//! Session-header Ship dialog: preview/host a frontend and/or deploy a contract.

use gpui::{KeyBinding, actions};

use super::*;
use waku_client::ship::{
    FrontendDetect, HostingProvider, HostingRecord, HostingTokenStatus, PreviewStatus,
    ShipHistoryItem,
};
use waku_client::web3::{
    DeployArtifact, DeploymentRecord, EvmNetwork, PfToolchainStatus, WalletAccount, WalletSource,
    explorer_address_url, faucet_url, short_digest,
};

actions!(waku_deploy_dialog, [DismissDeployDialog]);

const DIALOG_CONTEXT: &str = "DeployDialog";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        DismissDeployDialog,
        Some(DIALOG_CONTEXT),
    )]);
}

pub(super) struct DeployDialogState {
    cwd: PathBuf,
    artifacts: Option<Vec<DeployArtifact>>,
    networks: Option<Vec<EvmNetwork>>,
    wallets: Option<Vec<WalletAccount>>,
    frontend: Option<FrontendDetect>,
    preview: Option<PreviewStatus>,
    hosting_tokens: Option<Vec<HostingTokenStatus>>,
    history: Option<Vec<ShipHistoryItem>>,
    pf: Option<PfToolchainStatus>,
    selected_artifact: usize,
    selected_network: usize,
    selected_wallet: usize,
    ctor_sig: Entity<ComposerInput>,
    ctor_args: Entity<ComposerInput>,
    deploying: bool,
    hosting: bool,
    result: Option<DeploymentRecord>,
    hosting_result: Option<HostingRecord>,
    error: Option<String>,
    generation: u64,
    focus: FocusHandle,
}

impl Waku {
    pub(super) fn open_deploy_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let cwd = session
            .workspace
            .path()
            .map(Path::to_path_buf)
            .or_else(|| {
                self.state
                    .projects
                    .iter()
                    .find(|project| project.id == session.project_id)
                    .map(|project| project.path.clone())
            });
        let Some(cwd) = cwd else {
            self.show_toast(tr!("web3.deploy_no_workspace"));
            return;
        };
        let ctor_sig = cx.new(|cx| {
            ComposerInput::new(window, cx)
                .search_field()
                .placeholder(tr!("web3.ctor_sig_placeholder"))
        });
        let ctor_args = cx.new(|cx| {
            ComposerInput::new(window, cx)
                .search_field()
                .placeholder(tr!("web3.ctor_args_placeholder"))
        });
        self.deploy_dialog = Some(DeployDialogState {
            cwd: cwd.clone(),
            artifacts: None,
            networks: None,
            wallets: None,
            frontend: None,
            preview: None,
            hosting_tokens: None,
            history: None,
            pf: None,
            selected_artifact: 0,
            selected_network: 0,
            selected_wallet: 0,
            ctor_sig,
            ctor_args,
            deploying: false,
            hosting: false,
            result: None,
            hosting_result: None,
            error: None,
            generation: 1,
            focus: cx.focus_handle(),
        });
        self.reload_deploy_dialog(cx);
        cx.notify();
    }

    fn close_deploy_dialog(&mut self, cx: &mut Context<Self>) {
        self.deploy_dialog = None;
        cx.notify();
    }

    fn open_pf_settings_from_ship(&mut self, cx: &mut Context<Self>) {
        self.close_deploy_dialog(cx);
        self.open_settings_page(SettingsPage::ProofForge, cx);
    }

    fn reload_deploy_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.deploy_dialog.as_mut() else {
            return;
        };
        dialog.generation += 1;
        let generation = dialog.generation;
        let cwd = dialog.cwd.clone();
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { load_deploy_snapshot(&daemon, cwd) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(dialog) = this.deploy_dialog.as_mut() else {
                    return;
                };
                if dialog.generation != generation {
                    return;
                }
                match snapshot {
                    Ok(snapshot) => {
                        dialog.artifacts = Some(snapshot.artifacts);
                        dialog.networks = Some(snapshot.networks);
                        dialog.wallets = Some(snapshot.wallets);
                        dialog.frontend = Some(snapshot.frontend);
                        dialog.preview = Some(snapshot.preview);
                        dialog.hosting_tokens = Some(snapshot.hosting_tokens);
                        dialog.history = Some(snapshot.history);
                        dialog.pf = snapshot.pf;
                        dialog.error = None;
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send_deploy(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.deploy_dialog.as_mut() else {
            return;
        };
        if dialog.deploying {
            return;
        }
        let Some(artifact) = dialog
            .artifacts
            .as_ref()
            .and_then(|artifacts| artifacts.get(dialog.selected_artifact))
            .cloned()
        else {
            dialog.error = Some(tr!("web3.deploy_no_artifact"));
            cx.notify();
            return;
        };
        let Some(network) = dialog
            .networks
            .as_ref()
            .and_then(|networks| networks.get(dialog.selected_network))
            .cloned()
        else {
            dialog.error = Some(tr!("web3.deploy_no_network"));
            cx.notify();
            return;
        };
        let Some(wallet) = dialog
            .wallets
            .as_ref()
            .and_then(|wallets| wallets.get(dialog.selected_wallet))
            .cloned()
        else {
            dialog.error = Some(tr!("web3.deploy_no_wallet"));
            cx.notify();
            return;
        };
        let ctor_sig = dialog.ctor_sig.read(cx).content().trim().to_string();
        let ctor_args = dialog
            .ctor_args
            .read(cx)
            .content()
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let cwd = dialog.cwd.clone();
        dialog.deploying = true;
        dialog.error = None;
        dialog.generation += 1;
        let generation = dialog.generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        waku_client::Command::Web3DeploySend {
                            bin_path: artifact.bin_path,
                            module: artifact.module,
                            network_id: network.id,
                            wallet_id: wallet.id,
                            ctor_sig,
                            ctor_args,
                            digest: artifact.digest,
                            cwd: Some(cwd),
                        },
                    )? {
                        waku_client::ResponsePayload::Web3DeploySend { record } => Ok(record),
                        _ => anyhow::bail!("invalid deploy response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(dialog) = this.deploy_dialog.as_mut() else {
                    return;
                };
                if dialog.generation != generation {
                    return;
                }
                dialog.deploying = false;
                match result {
                    Ok(record) => {
                        dialog.result = Some(record.clone());
                        let url = dialog.networks.as_ref().and_then(|networks| {
                            networks
                                .iter()
                                .find(|network| network.id == record.network_id)
                                .and_then(|network| network.explorer_url.as_deref())
                                .map(|base| explorer_address_url(base, &record.address))
                        });
                        if let Some(history) = dialog.history.as_mut() {
                            history.insert(
                                0,
                                ShipHistoryItem {
                                    kind: "contract".into(),
                                    title: record.module.clone(),
                                    detail: record.address.clone(),
                                    ts: record.ts.clone(),
                                    url,
                                },
                            );
                        }
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn send_hosting_deploy(&mut self, provider: HostingProvider, cx: &mut Context<Self>) {
        let Some(dialog) = self.deploy_dialog.as_mut() else {
            return;
        };
        if dialog.hosting || dialog.deploying {
            return;
        }
        let ready = dialog.hosting_tokens.as_ref().is_some_and(|tokens| {
            tokens.iter().any(|token| {
                token.provider == provider.id() && token.configured && token.enabled
            })
        });
        if !ready {
            dialog.error = Some(tr!("ship.hosting_token_hint"));
            cx.notify();
            return;
        }
        let cwd = dialog.cwd.clone();
        dialog.hosting = true;
        dialog.error = None;
        dialog.generation += 1;
        let generation = dialog.generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        waku_client::Command::HostingDeploy { cwd, provider },
                    )? {
                        waku_client::ResponsePayload::HostingDeploy { record } => Ok(record),
                        _ => anyhow::bail!("invalid hosting deploy response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(dialog) = this.deploy_dialog.as_mut() else {
                    return;
                };
                if dialog.generation != generation {
                    return;
                }
                dialog.hosting = false;
                match result {
                    Ok(record) => {
                        dialog.hosting_result = Some(record.clone());
                        if let Some(history) = dialog.history.as_mut() {
                            history.insert(
                                0,
                                ShipHistoryItem {
                                    kind: "hosting".into(),
                                    title: format!("{} · {}", record.provider, record.project_name),
                                    detail: record.url.clone(),
                                    ts: record.ts.clone(),
                                    url: Some(record.url),
                                },
                            );
                        }
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_deploy_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.deploy_dialog.as_ref()?;
        let theme = Theme::current(cx);
        let artifacts = dialog.artifacts.clone().unwrap_or_default();
        let networks = dialog.networks.clone().unwrap_or_default();
        let wallets = dialog.wallets.clone().unwrap_or_default();
        let history = dialog.history.clone().unwrap_or_default();
        let frontend = dialog.frontend.clone();
        let preview = dialog.preview.clone();
        let hosting_tokens = dialog.hosting_tokens.clone().unwrap_or_default();
        let selected_artifact = dialog.selected_artifact;
        let selected_network = dialog.selected_network;
        let selected_wallet = dialog.selected_wallet;
        let deploying = dialog.deploying;
        let hosting = dialog.hosting;
        let error = dialog.error.clone();
        let result = dialog.result.clone();
        let hosting_result = dialog.hosting_result.clone();
        let pf_state = dialog.pf.as_ref().map(|status| status.state.clone());
        let ctor_sig = dialog.ctor_sig.clone();
        let ctor_args = dialog.ctor_args.clone();
        let focus = dialog.focus.clone();
        let selected_network_id = networks
            .get(selected_network)
            .map(|network| network.id.clone());
        let faucet = selected_network_id
            .as_deref()
            .and_then(faucet_url)
            .map(str::to_string);
        let cloudflare_ready = hosting_tokens.iter().any(|token| {
            token.provider == HostingProvider::Cloudflare.id() && token.configured && token.enabled
        });
        let vercel_ready = hosting_tokens.iter().any(|token| {
            token.provider == HostingProvider::Vercel.id() && token.configured && token.enabled
        });
        let frontend_kind = frontend
            .as_ref()
            .map(|detect| detect.kind.as_str())
            .unwrap_or("none");
        let can_host = frontend_kind != "none";

        let mut body = div()
            .id("deploy-dialog")
            .track_focus(&focus)
            .key_context(DIALOG_CONTEXT)
            .on_action(cx.listener(|this, _: &DismissDeployDialog, _, cx| {
                this.close_deploy_dialog(cx)
            }))
            .w(px(540.0))
            .max_h(px(720.0))
            .p(px(20.0))
            .rounded(px(16.0))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(12.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("ship.title")),
            )
            .child(hint_line(&theme, tr!("ship.description")));

        body = body
            .child(section_title(&theme, tr!("ship.frontend")))
            .child(hint_line(
                &theme,
                frontend
                    .as_ref()
                    .map(|detect| detect.hint.clone())
                    .unwrap_or_else(|| tr!("preview.scanning")),
            ));
        if let Some(url) = preview.as_ref().and_then(|status| status.url.clone()) {
            body = body.child(hint_line(&theme, url));
        }
        if let Some(record) = hosting_result {
            body = body.child(hint_line(&theme, record.url));
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    ship_action(&theme, "ship-preview", tr!("preview.button"), cx.listener(
                        |this, _, _, cx| this.start_local_preview(cx),
                    )),
                )
                .child(ship_action(
                    &theme,
                    "ship-cloudflare",
                    if hosting {
                        tr!("ship.hosting")
                    } else {
                        tr!("ship.cloudflare")
                    },
                    cx.listener(|this, _, _, cx| {
                        this.send_hosting_deploy(HostingProvider::Cloudflare, cx)
                    }),
                ).when(!can_host || !cloudflare_ready || hosting || deploying, |button| {
                    button.opacity(0.45)
                }))
                .child(ship_action(
                    &theme,
                    "ship-vercel",
                    tr!("ship.vercel"),
                    cx.listener(|this, _, _, cx| {
                        this.send_hosting_deploy(HostingProvider::Vercel, cx)
                    }),
                ).when(!can_host || !vercel_ready || hosting || deploying, |button| {
                    button.opacity(0.45)
                })),
        );
        if !cloudflare_ready && !vercel_ready {
            body = body.child(hint_line(&theme, tr!("ship.hosting_token_hint")));
        }

        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(section_title(&theme, tr!("ship.contract")))
                .child(ship_action(
                    &theme,
                    "ship-rescan",
                    tr!("web3.rescan"),
                    cx.listener(|this, _, _, cx| this.reload_deploy_dialog(cx)),
                )),
        );
        body = body.child(hint_line(&theme, tr!("web3.deploy_description")));
        if contract_needs_pf_install(pf_state.as_deref()) {
            body = body.child(link_line(
                &theme,
                "deploy-install-pf",
                tr!("web3.deploy_install_pf"),
                cx.listener(|this, _, _, cx| this.open_pf_settings_from_ship(cx)),
            ));
        }
        if artifacts.is_empty() {
            if !contract_needs_pf_install(pf_state.as_deref()) {
                body = body.child(hint_line(&theme, tr!("web3.deploy_no_artifact")));
            }
        } else {
            body = body.child(self.render_artifact_picker(
                &artifacts,
                selected_artifact,
                &theme,
                cx,
            ));
        }
        body = body.child(self.render_deploy_picker(
            "deploy-network",
            tr!("web3.network"),
            networks.iter().enumerate().map(|(index, network)| {
                (
                    index,
                    format!("{} ({})", network.name, network.chain_id),
                    index == selected_network,
                )
            }),
            |dialog, index| dialog.selected_network = index,
            &theme,
            cx,
        ));
        if let Some(url) = faucet {
            body = body.child(link_line(
                &theme,
                "deploy-faucet",
                tr!("web3.faucet"),
                move |_, _, cx| cx.open_url(&url),
            ));
        }
        body = body.child(self.render_deploy_picker(
            "deploy-wallet",
            tr!("web3.wallet"),
            wallets.iter().enumerate().map(|(index, wallet)| {
                (
                    index,
                    format!("{} · {}", wallet.label, wallet.address),
                    index == selected_wallet,
                )
            }),
            |dialog, index| dialog.selected_wallet = index,
            &theme,
            cx,
        ));
        body = body
            .child(TextField::new("deploy-ctor-sig", ctor_sig).w_full())
            .child(TextField::new("deploy-ctor-args", ctor_args).w_full());
        if let Some(error) = error {
            body = body.child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.danger)
                    .child(SharedString::from(error)),
            );
        }
        if let Some(record) = result {
            let explorer = networks
                .iter()
                .find(|network| network.id == record.network_id)
                .and_then(|network| network.explorer_url.as_deref())
                .map(|base| explorer_address_url(base, &record.address));
            body = body.child(hint_line(
                &theme,
                format!("{} · {}", record.address, record.tx_hash),
            ));
            if let Some(url) = explorer {
                body = body.child(link_line(
                    &theme,
                    "deploy-explorer",
                    tr!("web3.open_explorer"),
                    move |_, _, cx| cx.open_url(&url),
                ));
            }
        }
        if !history.is_empty() {
            body = body.child(section_title(&theme, tr!("ship.history")));
            for (index, item) in history.iter().take(8).enumerate() {
                let line = format!("{} · {} · {}", item.kind, item.title, item.detail);
                if let Some(url) = item.url.clone() {
                    body = body.child(link_line(
                        &theme,
                        ("deploy-history", index),
                        line,
                        move |_, _, cx| cx.open_url(&url),
                    ));
                } else {
                    body = body.child(hint_line(&theme, line));
                }
            }
        }
        body = body.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(
                    div()
                        .id("deploy-cancel")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(theme.accent))
                        .px(px(12.0))
                        .h(px(30.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .cursor_default()
                        .text_color(theme.text_secondary)
                        .child(tr!("common.cancel"))
                        .on_click(cx.listener(|this, _, _, cx| this.close_deploy_dialog(cx))),
                )
                .child(
                    div()
                        .id("deploy-send")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(theme.accent))
                        .px(px(12.0))
                        .h(px(30.0))
                        .rounded(px(8.0))
                        .bg(theme.inverse)
                        .flex()
                        .items_center()
                        .cursor_default()
                        .text_color(theme.on_inverse)
                        .child(if deploying {
                            SharedString::from(tr!("web3.deploying"))
                        } else {
                            SharedString::from(tr!("web3.deploy"))
                        })
                        .when(!deploying, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| this.send_deploy(cx)))
                        }),
                ),
        );

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("deploy-dialog-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_deploy_dialog(cx)),
            )
            .child(body);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    fn render_artifact_picker(
        &self,
        artifacts: &[DeployArtifact],
        selected: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().flex().flex_col().gap(px(4.0));
        for (index, artifact) in artifacts.iter().enumerate() {
            let digest = artifact.digest.as_deref().map(short_digest);
            list = list.child(
                div()
                    .id(("deploy-artifact", index))
                    .tab_index(0)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .px(px(10.0))
                    .h(px(28.0))
                    .rounded(px(7.0))
                    .bg(if index == selected {
                        theme.overlay
                    } else {
                        gpui::transparent_black()
                    })
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .cursor_default()
                    .text_size(px(12.5))
                    .text_color(theme.text)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(format!(
                                "{} · {}",
                                artifact.module, artifact.dir
                            ))),
                    )
                    .when_some(digest, |row, digest| {
                        row.child(
                            div()
                                .flex_none()
                                .px(px(6.0))
                                .rounded(px(5.0))
                                .bg(theme.overlay_strong)
                                .text_size(px(10.0))
                                .font_family("monospace")
                                .text_color(theme.text_secondary)
                                .child(SharedString::from(digest)),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(dialog) = this.deploy_dialog.as_mut() {
                            dialog.selected_artifact = index;
                            cx.notify();
                        }
                    })),
            );
        }
        div()
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_secondary)
                    .child(tr!("web3.artifact")),
            )
            .child(list)
    }

    fn render_deploy_picker(
        &self,
        id_prefix: &'static str,
        label: impl Into<SharedString>,
        items: impl IntoIterator<Item = (usize, String, bool)>,
        select: impl Fn(&mut DeployDialogState, usize) + Copy + 'static,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().flex().flex_col().gap(px(4.0));
        for (index, title, selected) in items {
            list = list.child(
                div()
                    .id((id_prefix, index))
                    .tab_index(0)
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .px(px(10.0))
                    .h(px(28.0))
                    .rounded(px(7.0))
                    .bg(if selected {
                        theme.overlay
                    } else {
                        gpui::transparent_black()
                    })
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(px(12.5))
                    .text_color(theme.text)
                    .child(SharedString::from(title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(dialog) = this.deploy_dialog.as_mut() {
                            select(dialog, index);
                            cx.notify();
                        }
                    })),
            );
        }
        div()
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_secondary)
                    .child(label.into()),
            )
            .child(list)
    }
}

struct ShipSnapshot {
    artifacts: Vec<DeployArtifact>,
    networks: Vec<EvmNetwork>,
    wallets: Vec<WalletAccount>,
    frontend: FrontendDetect,
    preview: PreviewStatus,
    hosting_tokens: Vec<HostingTokenStatus>,
    history: Vec<ShipHistoryItem>,
    pf: Option<PfToolchainStatus>,
}

fn load_deploy_snapshot(
    daemon: &waku_client::DaemonClient,
    cwd: PathBuf,
) -> anyhow::Result<ShipSnapshot> {
    let artifacts = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        waku_client::Command::Web3DeployScan { cwd: cwd.clone() },
    )? {
        waku_client::ResponsePayload::Web3DeployScan { artifacts } => artifacts,
        _ => anyhow::bail!("invalid deploy scan response"),
    };
    let networks = match daemon.request(Uuid::nil(), Uuid::nil(), waku_client::Command::Web3Networks)?
    {
        waku_client::ResponsePayload::Web3Networks { networks } => networks
            .into_iter()
            .filter(|network| network.enabled)
            .collect(),
        _ => anyhow::bail!("invalid networks response"),
    };
    let wallets = match daemon.request(Uuid::nil(), Uuid::nil(), waku_client::Command::Web3Wallets)? {
        waku_client::ResponsePayload::Web3Wallets { wallets } => wallets
            .into_iter()
            .filter(|wallet| matches!(wallet.source, WalletSource::Local | WalletSource::DevEnvKey))
            .collect(),
        _ => anyhow::bail!("invalid wallets response"),
    };
    let frontend = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        waku_client::Command::PreviewScan { cwd: cwd.clone() },
    )? {
        waku_client::ResponsePayload::PreviewScan { detect } => detect,
        _ => anyhow::bail!("invalid preview scan response"),
    };
    let preview = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        waku_client::Command::PreviewStatus { cwd: cwd.clone() },
    )? {
        waku_client::ResponsePayload::PreviewStatus { status } => status,
        _ => anyhow::bail!("invalid preview status response"),
    };
    let hosting_tokens =
        match daemon.request(Uuid::nil(), Uuid::nil(), waku_client::Command::HostingTokens)? {
            waku_client::ResponsePayload::HostingTokens { tokens } => tokens,
            _ => anyhow::bail!("invalid hosting tokens response"),
        };
    let history = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        waku_client::Command::ShipHistory { cwd: Some(cwd) },
    )? {
        waku_client::ResponsePayload::ShipHistory { items } => items,
        _ => anyhow::bail!("invalid ship history response"),
    };
    let pf = match daemon.request(Uuid::nil(), Uuid::nil(), waku_client::Command::PfStatus) {
        Ok(waku_client::ResponsePayload::PfStatus { status }) => Some(status),
        _ => None,
    };
    Ok(ShipSnapshot {
        artifacts,
        networks,
        wallets,
        frontend,
        preview,
        hosting_tokens,
        history,
        pf,
    })
}

pub(super) fn contract_needs_pf_install(pf_state: Option<&str>) -> bool {
    matches!(pf_state, Some("missing") | Some("installing"))
}

fn hint_line(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(theme.text_tertiary)
        .child(text.into())
}

fn link_line(
    theme: &Theme,
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .text_size(px(12.0))
        .text_color(theme.text_secondary)
        .cursor_default()
        .hover(|style| style.text_color(theme.text))
        .child(text.into())
        .on_click(on_click)
}

fn section_title(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(4.0))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text.into())
}

fn ship_action(
    theme: &Theme,
    id: &'static str,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .px(px(10.0))
        .h(px(28.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .items_center()
        .cursor_default()
        .text_size(px(12.0))
        .text_color(theme.text)
        .hover(|style| style.bg(theme.overlay))
        .child(label.into())
        .on_click(on_click)
}
