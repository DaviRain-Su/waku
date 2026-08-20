//! Settings → ProofForge. Daemon owns the compiler install; frames read cache.

use super::*;
use proofship_client::web3::PfToolchainStatus;

impl Waku {
    pub(super) fn ensure_pf_settings(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.pf_pending && !force {
            return;
        }
        if !force && self.pf_status.is_some() {
            return;
        }
        self.pf_pending = true;
        self.pf_generation += 1;
        let generation = self.pf_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::PfStatus,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pf_generation != generation {
                    return;
                }
                this.pf_pending = false;
                match result {
                    Ok(proofship_client::ResponsePayload::PfStatus { status }) => {
                        this.pf_status = Some(status.clone());
                        this.pf_error = None;
                        if status.state == "installing" {
                            this.poll_pf_install(cx);
                        }
                    }
                    Ok(_) => this.pf_error = Some("invalid ProofForge status".into()),
                    Err(error) => this.pf_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_pf_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let mut column = div().mt(px(15.0)).flex().flex_col().gap(px(12.0));
        if let Some(error) = &self.pf_error {
            column = column.child(pf_error(&theme, error));
        }
        column = column.child(
            pf_card(&theme)
                .child(pf_title(&theme, tr!("pf.description_title")))
                .child(pf_hint(&theme, tr!("pf.description"))),
        );
        match self.pf_status.as_ref() {
            Some(status) => column = column.child(self.render_pf_status_card(status, &theme, cx)),
            None => column = column.child(pf_hint(&theme, tr!("pf.loading"))),
        }
        column.into_any_element()
    }

    fn render_pf_status_card(
        &self,
        status: &PfToolchainStatus,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let state_label = match status.state.as_str() {
            "ready" => tr!("pf.state_ready"),
            "installing" => tr!("pf.state_installing"),
            _ => tr!("pf.state_missing"),
        };
        let source_label = match status.source.as_deref() {
            Some("host") => tr!("pf.source_host"),
            Some("managed") => tr!("pf.source_managed"),
            _ => String::new(),
        };
        let evm_label = if status.state == "ready" {
            if status.evm_ready {
                tr!("pf.evm_ready")
            } else {
                tr!("pf.evm_missing")
            }
        } else {
            String::new()
        };
        let detail = [
            status.version.clone().unwrap_or_default(),
            source_label,
            status.cli.clone().unwrap_or_default(),
            evm_label,
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        pf_card(theme).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(pf_title(theme, state_label))
                        .when(!detail.is_empty(), |column| {
                            column.child(pf_hint(theme, detail))
                        })
                        .when_some(status.error.clone(), |column, error| {
                            column.child(pf_error(theme, error))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .when(status.state == "missing", |row| {
                            row.child(
                                pf_button(theme, "pf-install", tr!("pf.install"))
                                    .on_click(cx.listener(|this, _, _, cx| this.install_pf(cx))),
                            )
                        })
                        .when(status.state == "installing", |row| {
                            row.child(pf_hint(theme, tr!("pf.installing")))
                        })
                        .when(
                            status.state == "ready" && status.source.as_deref() == Some("managed"),
                            |row| {
                                row.child(
                                    pf_button(theme, "pf-uninstall", tr!("pf.uninstall")).on_click(
                                        cx.listener(|this, _, _, cx| this.uninstall_pf(cx)),
                                    ),
                                )
                            },
                        ),
                ),
        )
    }

    fn install_pf(&mut self, cx: &mut Context<Self>) {
        self.pf_pending = true;
        self.pf_generation += 1;
        let generation = self.pf_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::PfInstall,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pf_generation != generation {
                    return;
                }
                this.pf_pending = false;
                match result {
                    Ok(proofship_client::ResponsePayload::PfStatus { status }) => {
                        this.pf_status = Some(status.clone());
                        this.pf_error = None;
                        if status.state == "installing" {
                            this.poll_pf_install(cx);
                        } else if status.state == "ready" {
                            this.show_toast(tr!("pf.ready_toast"));
                        }
                    }
                    Ok(_) => this.pf_error = Some("invalid ProofForge install response".into()),
                    Err(error) => this.pf_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn uninstall_pf(&mut self, cx: &mut Context<Self>) {
        self.pf_pending = true;
        self.pf_generation += 1;
        let generation = self.pf_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::PfUninstall,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pf_generation != generation {
                    return;
                }
                this.pf_pending = false;
                match result {
                    Ok(proofship_client::ResponsePayload::PfStatus { status }) => {
                        this.pf_status = Some(status);
                        this.pf_error = None;
                    }
                    Ok(_) => this.pf_error = Some("invalid ProofForge uninstall response".into()),
                    Err(error) => this.pf_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn poll_pf_install(&mut self, cx: &mut Context<Self>) {
        let daemon = self.daemon.client();
        let generation = self.pf_generation;
        cx.spawn(async move |this, cx| {
            for _ in 0..180 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let snapshot = cx
                    .background_executor()
                    .spawn({
                        let daemon = daemon.clone();
                        async move {
                            daemon.request(
                                Uuid::nil(),
                                Uuid::nil(),
                                proofship_client::Command::PfStatus,
                            )
                        }
                    })
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        if this.pf_generation != generation {
                            return true;
                        }
                        if let Ok(proofship_client::ResponsePayload::PfStatus { status }) = snapshot
                        {
                            this.pf_status = Some(status.clone());
                            this.pf_error = None;
                            cx.notify();
                            if status.state == "ready" {
                                this.show_toast(tr!("pf.ready_toast"));
                                return true;
                            }
                            return status.state != "installing";
                        }
                        false
                    })
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        })
        .detach();
    }
}

fn pf_card(theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(20.0))
        .py(px(14.0))
        .rounded(px(13.0))
        .bg(theme.raised)
}

fn pf_title(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text.into())
}

fn pf_hint(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(4.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .text_color(theme.text_tertiary)
        .child(text.into())
}

fn pf_error(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(theme.danger)
        .child(text.into())
}

fn pf_button(
    theme: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .px(px(10.0))
        .h(px(28.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.0))
        .text_color(theme.text)
        .cursor_default()
        .hover(|style| style.bg(theme.overlay))
        .child(label.into())
}
