//! Desktop proxy for the provider runtime owned by `proofship-daemon`.

use std::sync::Arc;

use crate::computer_use::ComputerToolRequest;
use crate::model::{
    BackgroundWorkKey, DriverEvent, ProviderKind, ProviderResumeCursor, RuntimeEventCursor,
};

pub use proofship_client::driver::{
    DriverControl, DriverEventSender, DriverHandle, DriverStartOptions, SessionOptions,
    event_channel,
};

pub(crate) fn start_remote(
    client: proofship_client::DaemonClient,
    session_id: uuid::Uuid,
    provider: ProviderKind,
    options: DriverStartOptions,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    let runtime_id = uuid::Uuid::new_v4();
    let command = proofship_client::Command::Start {
        options: proofship_client::WireDriverStartOptions {
            provider: proofship_client::encode_enum(provider)?,
            binary: options.binary,
            cwd: options.cwd,
            mode: proofship_client::encode_enum(options.mode)?,
            interaction_mode: proofship_client::encode_enum(options.interaction_mode)?,
            model: options.model,
            reasoning_effort: options.reasoning_effort,
            service_tier: options.service_tier,
            context_window: options.context_window,
            agent_preset: options.agent_preset,
            computer_use_enabled: options.computer_use_enabled,
            provider_cursor: options
                .provider_cursor
                .map(serde_json::to_value)
                .transpose()?,
        },
    };
    let supports_steer = match client.request(session_id, runtime_id, command) {
        Ok(proofship_client::ResponsePayload::Started { supports_steer }) => supports_steer,
        Ok(_) => anyhow::bail!("Waku daemon returned an invalid start response"),
        Err(error) => return Err(error),
    };
    connect_remote(client, session_id, runtime_id, supports_steer, None, events)
}

pub(crate) fn attach_remote(
    client: proofship_client::DaemonClient,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    replay_cursor: Option<RuntimeEventCursor>,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    connect_remote(
        client,
        session_id,
        runtime_id,
        supports_steer,
        replay_cursor,
        events,
    )
}

fn connect_remote(
    client: proofship_client::DaemonClient,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    replay_cursor: Option<RuntimeEventCursor>,
    events: DriverEventSender,
) -> anyhow::Result<DriverHandle> {
    let remote_events = client.subscribe(session_id, runtime_id);
    let forwarding_events = events.clone();
    let thread_client = client.clone();
    let spawn = std::thread::Builder::new()
        .name(format!("proofship-daemon-session-{session_id}"))
        .spawn(move || {
            let mut saw_process_exit = false;
            while let Ok(sequenced) = remote_events.recv() {
                if replay_cursor.is_some_and(|cursor| {
                    cursor.runtime_id == sequenced.runtime_id
                        && cursor.epoch == sequenced.epoch
                        && cursor.sequence >= sequenced.sequence
                }) {
                    continue;
                }
                let cursor = RuntimeEventCursor {
                    runtime_id: sequenced.runtime_id,
                    epoch: sequenced.epoch,
                    sequence: sequenced.sequence,
                };
                let event = match proofship_client::event_from_wire(sequenced.event) {
                    Ok(event) => event,
                    Err(error) => {
                        DriverEvent::Error(format!("Waku daemon sent an invalid event: {error}"))
                    }
                };
                saw_process_exit |= matches!(&event, DriverEvent::ProcessExited);
                if forwarding_events.send(event).is_err()
                    || forwarding_events
                        .send(DriverEvent::RuntimeEventCursorAdvanced(cursor))
                        .is_err()
                {
                    break;
                }
            }
            thread_client.unsubscribe(session_id, runtime_id);
            // A development hot-swap (or daemon crash) closes the old event
            // channel. Surface that as an ordinary provider exit so the task
            // cannot remain stuck in Working while the supervisor connects
            // subsequent work to the replacement daemon.
            if !saw_process_exit {
                let _ = forwarding_events.send(DriverEvent::ProcessExited);
            }
        });
    if let Err(error) = spawn {
        client.unsubscribe(session_id, runtime_id);
        return Err(error.into());
    }
    Ok(DriverHandle::from_control(Arc::new(RemoteDriverControl {
        client,
        session_id,
        runtime_id,
        supports_steer,
        events,
    })))
}

struct RemoteDriverControl {
    client: proofship_client::DaemonClient,
    session_id: uuid::Uuid,
    runtime_id: uuid::Uuid,
    supports_steer: bool,
    events: DriverEventSender,
}

impl RemoteDriverControl {
    fn notify(&self, command: proofship_client::Command) {
        if let Err(error) = self
            .client
            .notify(self.session_id, self.runtime_id, command)
        {
            let _ = self.events.send(DriverEvent::Error(format!(
                "Waku daemon command failed: {error}"
            )));
        }
    }
}

impl DriverControl for RemoteDriverControl {
    fn prompt(&self, prompt: String) {
        self.notify(proofship_client::Command::Prompt { prompt });
    }

    fn supports_steer(&self) -> bool {
        self.supports_steer
    }

    fn steer(&self, prompt: String) {
        self.notify(proofship_client::Command::Steer { prompt });
    }

    fn cancel(&self) {
        self.notify(proofship_client::Command::Cancel);
    }

    fn cancel_computer_use(&self) {
        self.notify(proofship_client::Command::CancelComputerUse);
    }

    fn refresh_background_work(&self) {
        self.notify(proofship_client::Command::RefreshBackgroundWork);
    }

    fn stop_background_work(&self, key: BackgroundWorkKey, control_id: String) {
        match serde_json::to_value(key) {
            Ok(key) => {
                self.notify(proofship_client::Command::StopBackgroundWork { key, control_id })
            }
            Err(error) => {
                let _ = self.events.send(DriverEvent::Error(format!(
                    "could not encode background-work command: {error}"
                )));
            }
        }
    }

    fn respond(&self, request_id: String, option_id: String) {
        self.notify(proofship_client::Command::Respond {
            request_id,
            option_id,
        });
    }

    fn respond_user_input(
        &self,
        request_id: String,
        answers: Vec<proofship_protocol::model::UserInputAnswer>,
    ) {
        self.notify(proofship_client::Command::RespondUserInput {
            request_id,
            answers,
        });
    }

    fn run_computer_tool(&self, request: ComputerToolRequest) {
        self.notify(proofship_client::Command::RunComputerTool {
            request: proofship_client::WireComputerToolRequest {
                call_id: request.call_id,
                tool: request.tool,
                arguments: request.arguments,
            },
        });
    }

    fn reject_computer_tool(&self, request: ComputerToolRequest, reason: String) {
        self.notify(proofship_client::Command::RejectComputerTool {
            request: proofship_client::WireComputerToolRequest {
                call_id: request.call_id,
                tool: request.tool,
                arguments: request.arguments,
            },
            reason,
        });
    }

    fn apply_options(&self, options: SessionOptions) -> bool {
        let options = (|| {
            Ok::<_, anyhow::Error>(proofship_client::WireSessionOptions {
                mode: proofship_client::encode_enum(options.mode)?,
                interaction_mode: proofship_client::encode_enum(options.interaction_mode)?,
                model: options.model,
                reasoning_effort: options.reasoning_effort,
                service_tier: options.service_tier,
                context_window: options.context_window,
            })
        })();
        let Ok(options) = options else {
            return false;
        };
        matches!(
            self.client.request(
                self.session_id,
                self.runtime_id,
                proofship_client::Command::ApplyOptions { options }
            ),
            Ok(proofship_client::ResponsePayload::OptionsApplied { applied: true })
        )
    }

    fn rollback(&self, turns: usize) -> anyhow::Result<Option<ProviderResumeCursor>> {
        match self.client.request(
            self.session_id,
            self.runtime_id,
            proofship_client::Command::Rollback { turns },
        )? {
            proofship_client::ResponsePayload::Cursor { cursor } => cursor
                .map(serde_json::from_value)
                .transpose()
                .map_err(Into::into),
            _ => anyhow::bail!("Waku daemon returned an invalid rollback response"),
        }
    }

    fn fork(&self, turns_to_remove: usize) -> anyhow::Result<ProviderResumeCursor> {
        match self.client.request(
            self.session_id,
            self.runtime_id,
            proofship_client::Command::Fork { turns_to_remove },
        )? {
            proofship_client::ResponsePayload::Cursor {
                cursor: Some(cursor),
            } => serde_json::from_value(cursor).map_err(Into::into),
            _ => anyhow::bail!("Waku daemon returned an invalid fork response"),
        }
    }

    fn close(&self) {
        self.notify(proofship_client::Command::CloseSession);
    }
}

impl Drop for RemoteDriverControl {
    fn drop(&mut self) {
        self.client.unsubscribe(self.session_id, self.runtime_id);
    }
}
