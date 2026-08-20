//! Settings → Networks and Wallets. Daemon owns the files; frames read cache.

use super::*;
use proofship_client::web3::{
    EvmNetwork, OkxStatus, WalletAccount, WalletBalanceSnapshot, WalletSource, Web3Prefs,
};

#[derive(Clone, Copy)]
pub(super) enum WalletDialogKind {
    Watch,
    DevEnvKey,
    Import,
    Create,
}

pub(super) struct NetworkDialog {
    editing_id: Option<String>,
    name: Entity<TextInput>,
    chain_id: Entity<TextInput>,
    rpc_url: Entity<TextInput>,
    explorer_url: Entity<TextInput>,
    symbol: Entity<TextInput>,
}

pub(super) struct WalletDialog {
    kind: WalletDialogKind,
    label: Entity<TextInput>,
    second: Entity<TextInput>,
}

impl Waku {
    pub(super) fn ensure_web3_settings(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.web3_pending && !force {
            return;
        }
        if !force
            && self.web3_networks.is_some()
            && self.web3_wallets.is_some()
            && self.web3_okx.is_some()
            && self.web3_prefs.is_some()
        {
            return;
        }
        self.web3_pending = true;
        self.web3_generation += 1;
        let generation = self.web3_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { load_web3_snapshot(&daemon) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.web3_generation != generation {
                    return;
                }
                this.web3_pending = false;
                match snapshot {
                    Ok((networks, wallets, okx, prefs)) => {
                        this.web3_networks = Some(networks);
                        this.web3_wallets = Some(wallets);
                        this.web3_okx = Some(okx);
                        this.web3_prefs = Some(prefs);
                        this.web3_error = None;
                        this.refresh_wallet_balances(cx);
                    }
                    Err(error) => this.web3_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_networks_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let mut column = div().mt(px(15.0)).flex().flex_col().gap(px(12.0));
        if let Some(error) = &self.web3_error {
            column = column.child(settings_error(&theme, error));
        }
        column = column.child(self.render_okx_card(&theme, cx));
        if let Some(networks) = &self.web3_networks {
            for (index, network) in networks.iter().enumerate() {
                column = column.child(self.render_network_row(index, network, &theme, cx));
            }
        } else {
            column = column.child(settings_hint(&theme, tr!("web3.loading")));
        }
        column = column.child(
            settings_button(&theme, "web3-add-network", tr!("web3.add_network")).on_click(
                cx.listener(|this, _, window, cx| this.open_network_dialog(None, window, cx)),
            ),
        );
        if let Some(dialog) = &self.web3_network_dialog {
            column = column.child(self.render_network_dialog(dialog, &theme, cx));
        }
        column.into_any_element()
    }

    pub(super) fn render_wallets_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let mut column = div().mt(px(15.0)).flex().flex_col().gap(px(12.0));
        if let Some(error) = &self.web3_error {
            column = column.child(settings_error(&theme, error));
        }
        column = column.child(self.render_active_network_picker(&theme, cx));
        if let Some(backup) = &self.web3_backup_hex {
            column = column.child(
                settings_card(&theme)
                    .child(settings_title(&theme, tr!("web3.backup_key")))
                    .child(settings_hint(&theme, tr!("web3.backup_key_description")))
                    .child(
                        div()
                            .mt(px(8.0))
                            .text_size(px(12.0))
                            .font_family("monospace")
                            .text_color(theme.text)
                            .child(SharedString::from(backup.clone())),
                    ),
            );
        }
        if let Some(wallets) = &self.web3_wallets {
            for (index, wallet) in wallets.iter().enumerate() {
                column = column.child(self.render_wallet_row(index, wallet, &theme, cx));
            }
        } else {
            column = column.child(settings_hint(&theme, tr!("web3.loading")));
        }
        column =
            column.child(
                div().flex().flex_wrap().gap(px(8.0)).children([
                    settings_button(
                        &theme,
                        "web3-refresh-balances",
                        tr!("web3.refresh_balances"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_wallet_balances(cx))),
                    settings_button(&theme, "web3-create-wallet", tr!("web3.create_wallet"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_wallet_dialog(WalletDialogKind::Create, window, cx)
                        })),
                    settings_button(&theme, "web3-import-wallet", tr!("web3.import_wallet"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_wallet_dialog(WalletDialogKind::Import, window, cx)
                        })),
                    settings_button(&theme, "web3-watch-wallet", tr!("web3.watch_wallet"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_wallet_dialog(WalletDialogKind::Watch, window, cx)
                        })),
                    settings_button(&theme, "web3-env-wallet", tr!("web3.env_wallet")).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_wallet_dialog(WalletDialogKind::DevEnvKey, window, cx)
                        }),
                    ),
                ]),
            );
        if let Some(dialog) = &self.web3_wallet_dialog {
            column = column.child(self.render_wallet_dialog(dialog, &theme, cx));
        }
        column.into_any_element()
    }

    pub(super) fn selected_network_id(&self) -> String {
        self.web3_prefs
            .as_ref()
            .map(|prefs| prefs.selected_network_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| proofship_client::web3::default_network_id().to_string())
    }

    pub(super) fn selected_network(&self) -> Option<EvmNetwork> {
        let id = self.selected_network_id();
        self.web3_networks
            .as_ref()?
            .iter()
            .find(|network| network.id == id)
            .cloned()
    }

    fn render_active_network_picker(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let selected = self.selected_network_id();
        let networks = self.web3_networks.clone().unwrap_or_default();
        let detail = networks
            .iter()
            .find(|network| network.id == selected)
            .or_else(|| networks.first())
            .map(|network| format!("Chain {} · {}", network.chain_id, network.rpc_url));
        let mut card =
            settings_card(theme)
                .child(settings_title(theme, tr!("web3.active_network")))
                .child(settings_hint(theme, tr!("web3.active_network_hint")))
                .child(div().mt(px(10.0)).flex().flex_wrap().gap(px(8.0)).children(
                    networks.into_iter().enumerate().map(|(index, network)| {
                        let id = network.id.clone();
                        let on = id == selected;
                        settings_button(
                            theme,
                            ("web3-select-network", index),
                            format!("{} ({})", network.name, network.chain_id),
                        )
                        .when(on, |button| {
                            button.bg(theme.inverse).text_color(theme.on_inverse)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_selected_network(id.clone(), cx)
                        }))
                    }),
                ));
        if let Some(detail) = detail {
            card = card.child(settings_hint(theme, detail));
        }
        card
    }

    pub(super) fn set_selected_network(&mut self, id: String, cx: &mut Context<Self>) {
        let prefs = Web3Prefs {
            selected_network_id: id,
        };
        self.web3_call(
            proofship_client::Command::Web3SetPrefs { prefs },
            cx,
            |this, payload, cx| {
                if let proofship_client::ResponsePayload::Web3Prefs { prefs } = payload {
                    this.web3_prefs = Some(prefs);
                    this.web3_balances = None;
                    this.refresh_wallet_balances(cx);
                }
            },
        );
    }

    fn render_okx_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let status = self.web3_okx.clone().unwrap_or_default();
        let enabled = status.enabled;
        settings_card(theme)
            .child(settings_title(theme, tr!("web3.okx_title")))
            .child(settings_hint(theme, tr!("web3.okx_description")))
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        TextField::new("web3-okx-key", self.web3_okx_input.clone())
                            .flex_1()
                            .max_w(px(360.0)),
                    )
                    .child(
                        settings_button(theme, "web3-okx-save", tr!("web3.save_key"))
                            .on_click(cx.listener(|this, _, _, cx| this.save_okx_key(cx))),
                    ),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(settings_hint(
                        theme,
                        status
                            .key_hint
                            .clone()
                            .unwrap_or_else(|| tr!("web3.okx_not_configured")),
                    ))
                    .child(render_toggle(
                        theme,
                        "web3-okx-enabled",
                        enabled,
                        cx.listener(move |this, _, _, cx| this.set_okx_enabled(!enabled, cx)),
                    )),
            )
    }

    fn render_network_row(
        &self,
        index: usize,
        network: &EvmNetwork,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let network = network.clone();
        let enabled = network.enabled;
        settings_card(theme)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(settings_title(theme, network.name.clone()))
                    .when(network.builtin, |row| {
                        row.child(settings_hint(theme, tr!("web3.builtin")))
                    })
                    .child(div().flex_1())
                    .child(render_toggle(
                        theme,
                        ("web3-network-enabled", index),
                        enabled,
                        cx.listener({
                            let network = network.clone();
                            move |this, _, _, cx| {
                                let mut network = network.clone();
                                network.enabled = !network.enabled;
                                this.upsert_network(network, cx);
                            }
                        }),
                    )),
            )
            .child(settings_hint(
                theme,
                format!("Chain {} · {}", network.chain_id, network.rpc_url),
            ))
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .gap(px(8.0))
                    .child(
                        settings_button(theme, ("web3-network-edit", index), tr!("web3.edit"))
                            .on_click(cx.listener({
                                let network = network.clone();
                                move |this, _, window, cx| {
                                    this.open_network_dialog(Some(network.clone()), window, cx);
                                }
                            })),
                    )
                    .when(!network.builtin, |row| {
                        row.child(
                            settings_button(
                                theme,
                                ("web3-network-remove", index),
                                tr!("web3.remove"),
                            )
                            .on_click(cx.listener({
                                let id = network.id.clone();
                                move |this, _, _, cx| this.remove_network(id.clone(), cx)
                            })),
                        )
                    }),
            )
    }

    fn render_wallet_row(
        &self,
        index: usize,
        wallet: &WalletAccount,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let source = match wallet.source {
            WalletSource::Local => tr!("web3.source_local"),
            WalletSource::Watch => tr!("web3.source_watch"),
            WalletSource::DevEnvKey => tr!("web3.source_env"),
            WalletSource::WalletConnect => tr!("web3.source_walletconnect"),
        };
        let detail = if wallet.address.is_empty() {
            wallet.env_key_name.clone().unwrap_or_default()
        } else {
            wallet.address.clone()
        };
        let snapshot = self.wallet_balance_snapshot(&wallet.id);
        settings_card(theme)
            .child(settings_title(theme, wallet.label.clone()))
            .child(settings_hint(theme, format!("{source} · {detail}")))
            .child(self.render_wallet_balances(wallet, snapshot.as_ref(), theme))
            .child(
                settings_button(theme, ("web3-wallet-remove", index), tr!("web3.remove")).on_click(
                    cx.listener({
                        let id = wallet.id.clone();
                        move |this, _, _, cx| this.remove_wallet(id.clone(), cx)
                    }),
                ),
            )
    }

    fn wallet_balance_snapshot(&self, wallet_id: &str) -> Option<WalletBalanceSnapshot> {
        self.web3_balances
            .as_ref()?
            .iter()
            .find(|row| row.wallet_id == wallet_id)
            .cloned()
    }

    fn render_wallet_balances(
        &self,
        wallet: &WalletAccount,
        snapshot: Option<&WalletBalanceSnapshot>,
        theme: &Theme,
    ) -> Div {
        if wallet.address.trim().is_empty() {
            return settings_hint(theme, tr!("web3.balance_no_address"));
        }
        match snapshot {
            None => settings_hint(theme, tr!("web3.balance_loading")),
            Some(snapshot) if snapshot.balances.is_empty() => {
                settings_hint(theme, tr!("web3.balance_no_network"))
            }
            Some(snapshot) => {
                let line = snapshot
                    .balances
                    .iter()
                    .map(|balance| {
                        if balance.error.is_some() {
                            format!("{} —", balance.network_name)
                        } else {
                            format!("{} {}", balance.display, balance.symbol)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                settings_hint(theme, line)
            }
        }
    }

    fn render_network_dialog(
        &self,
        dialog: &NetworkDialog,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        settings_card(theme)
            .child(settings_title(
                theme,
                if dialog.editing_id.is_some() {
                    tr!("web3.edit_network")
                } else {
                    tr!("web3.add_network")
                },
            ))
            .child(form_field("web3-net-name", dialog.name.clone()))
            .child(form_field("web3-net-chain", dialog.chain_id.clone()))
            .child(form_field("web3-net-rpc", dialog.rpc_url.clone()))
            .child(form_field("web3-net-explorer", dialog.explorer_url.clone()))
            .child(form_field("web3-net-symbol", dialog.symbol.clone()))
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .gap(px(8.0))
                    .child(
                        settings_button(theme, "web3-net-save", tr!("web3.save"))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_network_dialog(cx))),
                    )
                    .child(
                        settings_button(theme, "web3-net-cancel", tr!("common.cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.web3_network_dialog = None;
                                cx.notify();
                            }),
                        ),
                    ),
            )
    }

    fn render_wallet_dialog(
        &self,
        dialog: &WalletDialog,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let (title, hint) = wallet_dialog_copy(dialog.kind);
        settings_card(theme)
            .child(settings_title(theme, title))
            .child(settings_hint(theme, hint))
            .child(form_field("web3-wallet-label", dialog.label.clone()))
            .when(!matches!(dialog.kind, WalletDialogKind::Create), |card| {
                card.child(form_field("web3-wallet-second", dialog.second.clone()))
            })
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .gap(px(8.0))
                    .child(
                        settings_button(theme, "web3-wallet-save", tr!("web3.save"))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_wallet_dialog(cx))),
                    )
                    .child(
                        settings_button(theme, "web3-wallet-cancel", tr!("common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.web3_wallet_dialog = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn open_network_dialog(
        &mut self,
        existing: Option<EvmNetwork>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| TextInput::new(window, cx));
        let chain_id = cx.new(|cx| TextInput::new(window, cx));
        let rpc_url = cx.new(|cx| TextInput::new(window, cx));
        let explorer_url = cx.new(|cx| TextInput::new(window, cx));
        let symbol = cx.new(|cx| TextInput::new(window, cx));
        if let Some(network) = &existing {
            name.update(cx, |input, cx| input.set_content(network.name.clone(), cx));
            chain_id.update(cx, |input, cx| {
                input.set_content(network.chain_id.to_string(), cx)
            });
            rpc_url.update(cx, |input, cx| {
                input.set_content(network.rpc_url.clone(), cx)
            });
            explorer_url.update(cx, |input, cx| {
                input.set_content(network.explorer_url.clone().unwrap_or_default(), cx)
            });
            symbol.update(cx, |input, cx| {
                input.set_content(network.currency_symbol.clone(), cx)
            });
        }
        self.web3_network_dialog = Some(NetworkDialog {
            editing_id: existing.map(|network| network.id),
            name,
            chain_id,
            rpc_url,
            explorer_url,
            symbol,
        });
        cx.notify();
    }

    fn open_wallet_dialog(
        &mut self,
        kind: WalletDialogKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let second_placeholder = match kind {
            WalletDialogKind::Create => String::new(),
            WalletDialogKind::Import => tr!("web3.import_wallet_placeholder"),
            WalletDialogKind::Watch => tr!("web3.watch_wallet_placeholder"),
            WalletDialogKind::DevEnvKey => tr!("web3.env_wallet_placeholder"),
        };
        self.web3_wallet_dialog = Some(WalletDialog {
            kind,
            label: cx.new(|cx| {
                TextInput::new(window, cx).placeholder(tr!("web3.wallet_label_placeholder"))
            }),
            second: cx.new(|cx| TextInput::new(window, cx).placeholder(second_placeholder)),
        });
        cx.notify();
    }

    fn submit_network_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.web3_network_dialog else {
            return;
        };
        let name = dialog.name.read(cx).content().trim().to_string();
        let chain_id = dialog.chain_id.read(cx).content().trim().to_string();
        let rpc_url = dialog.rpc_url.read(cx).content().trim().to_string();
        let explorer = dialog.explorer_url.read(cx).content().trim().to_string();
        let symbol = dialog.symbol.read(cx).content().trim().to_string();
        let editing_id = dialog.editing_id.clone();
        let Ok(chain_id) = chain_id.parse::<u64>() else {
            self.web3_error = Some(tr!("web3.invalid_chain_id"));
            cx.notify();
            return;
        };
        let id = editing_id.clone().unwrap_or_else(|| slug(&name));
        let existing = self
            .web3_networks
            .as_ref()
            .and_then(|networks| networks.iter().find(|network| network.id == id));
        let network = EvmNetwork {
            id,
            name,
            chain_id,
            rpc_url,
            explorer_url: (!explorer.is_empty()).then_some(explorer),
            currency_symbol: if symbol.is_empty() {
                "ETH".into()
            } else {
                symbol
            },
            builtin: existing.is_some_and(|network| network.builtin),
            enabled: existing.is_none_or(|network| network.enabled),
        };
        self.web3_network_dialog = None;
        self.upsert_network(network, cx);
    }

    fn submit_wallet_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.web3_wallet_dialog else {
            return;
        };
        let kind = dialog.kind;
        let label = dialog.label.read(cx).content().trim().to_string();
        let second = dialog.second.read(cx).content().trim().to_string();
        if label.is_empty() {
            self.web3_error = Some(tr!("web3.wallet_label_empty"));
            cx.notify();
            return;
        }
        if matches!(kind, WalletDialogKind::Import) && second.is_empty() {
            self.web3_error = Some(tr!("web3.import_key_empty"));
            cx.notify();
            return;
        }
        if matches!(kind, WalletDialogKind::Watch) && second.is_empty() {
            self.web3_error = Some(tr!("web3.watch_address_empty"));
            cx.notify();
            return;
        }
        if matches!(kind, WalletDialogKind::DevEnvKey) && second.is_empty() {
            self.web3_error = Some(tr!("web3.env_name_empty"));
            cx.notify();
            return;
        }
        self.web3_wallet_dialog = None;
        let daemon = self.daemon.client();
        self.web3_pending = true;
        self.web3_generation += 1;
        let generation = self.web3_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match kind {
                        WalletDialogKind::Create => match daemon.request(
                            Uuid::nil(),
                            Uuid::nil(),
                            proofship_client::Command::Web3CreateWallet { label },
                        )? {
                            proofship_client::ResponsePayload::Web3WalletCreated { created } => {
                                Ok((created.wallets, Some(created.backup_hex)))
                            }
                            _ => anyhow::bail!("invalid create-wallet response"),
                        },
                        WalletDialogKind::Import => match daemon.request(
                            Uuid::nil(),
                            Uuid::nil(),
                            proofship_client::Command::Web3ImportWallet {
                                label,
                                secret: second,
                            },
                        )? {
                            proofship_client::ResponsePayload::Web3Wallets { wallets } => {
                                Ok((wallets, None))
                            }
                            _ => anyhow::bail!("invalid import-wallet response"),
                        },
                        WalletDialogKind::Watch | WalletDialogKind::DevEnvKey => {
                            let wallet = WalletAccount {
                                id: format!(
                                    "{}-{}",
                                    if matches!(kind, WalletDialogKind::Watch) {
                                        "watch"
                                    } else {
                                        "env"
                                    },
                                    &Uuid::new_v4().to_string()[..8]
                                ),
                                label,
                                address: if matches!(kind, WalletDialogKind::Watch) {
                                    second.clone()
                                } else {
                                    String::new()
                                },
                                source: if matches!(kind, WalletDialogKind::Watch) {
                                    WalletSource::Watch
                                } else {
                                    WalletSource::DevEnvKey
                                },
                                env_key_name: matches!(kind, WalletDialogKind::DevEnvKey)
                                    .then_some(second),
                            };
                            match daemon.request(
                                Uuid::nil(),
                                Uuid::nil(),
                                proofship_client::Command::Web3UpsertWallet { wallet },
                            )? {
                                proofship_client::ResponsePayload::Web3Wallets { wallets } => {
                                    Ok((wallets, None))
                                }
                                _ => anyhow::bail!("invalid upsert-wallet response"),
                            }
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.web3_generation != generation {
                    return;
                }
                this.web3_pending = false;
                match result {
                    Ok((wallets, backup)) => {
                        this.web3_wallets = Some(wallets);
                        this.web3_backup_hex = backup;
                        this.web3_error = None;
                        this.refresh_wallet_balances(cx);
                    }
                    Err(error) => this.web3_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn upsert_network(&mut self, network: EvmNetwork, cx: &mut Context<Self>) {
        self.web3_call(
            proofship_client::Command::Web3UpsertNetwork { network },
            cx,
            |this, payload, cx| {
                if let proofship_client::ResponsePayload::Web3Networks { networks } = payload {
                    this.web3_networks = Some(networks);
                    this.refresh_wallet_balances(cx);
                }
            },
        );
    }

    fn remove_network(&mut self, id: String, cx: &mut Context<Self>) {
        self.web3_call(
            proofship_client::Command::Web3RemoveNetwork { id },
            cx,
            |this, payload, cx| {
                if let proofship_client::ResponsePayload::Web3Networks { networks } = payload {
                    this.web3_networks = Some(networks);
                    this.refresh_wallet_balances(cx);
                }
            },
        );
    }

    fn remove_wallet(&mut self, id: String, cx: &mut Context<Self>) {
        self.web3_call(
            proofship_client::Command::Web3RemoveWallet { id },
            cx,
            |this, payload, cx| {
                if let proofship_client::ResponsePayload::Web3Wallets { wallets } = payload {
                    this.web3_wallets = Some(wallets);
                    this.refresh_wallet_balances(cx);
                }
            },
        );
    }

    fn refresh_wallet_balances(&mut self, cx: &mut Context<Self>) {
        let Some(wallets) = &self.web3_wallets else {
            return;
        };
        if !wallets
            .iter()
            .any(|wallet| !wallet.address.trim().is_empty())
        {
            self.web3_balances = Some(Vec::new());
            cx.notify();
            return;
        }
        self.web3_balances_generation += 1;
        let generation = self.web3_balances_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::Web3WalletBalances { wallet_id: None },
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.web3_balances_generation != generation {
                    return;
                }
                match result {
                    Ok(proofship_client::ResponsePayload::Web3WalletBalances { wallets }) => {
                        this.web3_balances = Some(wallets);
                    }
                    Ok(_) => this.web3_error = Some(tr!("web3.balance_invalid")),
                    Err(error) => this.web3_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_okx_key(&mut self, cx: &mut Context<Self>) {
        let api_key = self.web3_okx_input.read(cx).content().to_string();
        self.web3_okx_input
            .update(cx, |input, cx| input.set_content(String::new(), cx));
        self.web3_call(
            proofship_client::Command::Web3SetOkx {
                api_key: Some(api_key),
                enabled: None,
            },
            cx,
            |this, payload, _cx| {
                if let proofship_client::ResponsePayload::Web3OkxStatus { status } = payload {
                    this.web3_okx = Some(status);
                }
            },
        );
    }

    fn set_okx_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.web3_call(
            proofship_client::Command::Web3SetOkx {
                api_key: None,
                enabled: Some(enabled),
            },
            cx,
            |this, payload, _cx| {
                if let proofship_client::ResponsePayload::Web3OkxStatus { status } = payload {
                    this.web3_okx = Some(status);
                }
            },
        );
    }

    fn web3_call(
        &mut self,
        command: proofship_client::Command,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Self, proofship_client::ResponsePayload, &mut Context<Self>)
        + Send
        + 'static,
    ) {
        let daemon = self.daemon.client();
        self.web3_generation += 1;
        let generation = self.web3_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon.request(Uuid::nil(), Uuid::nil(), command) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.web3_generation != generation {
                    return;
                }
                match result {
                    Ok(payload) => {
                        apply(this, payload, cx);
                        this.web3_error = None;
                    }
                    Err(error) => this.web3_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn load_web3_snapshot(
    daemon: &proofship_client::DaemonClient,
) -> anyhow::Result<(Vec<EvmNetwork>, Vec<WalletAccount>, OkxStatus, Web3Prefs)> {
    let networks = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        proofship_client::Command::Web3Networks,
    )? {
        proofship_client::ResponsePayload::Web3Networks { networks } => networks,
        _ => anyhow::bail!("invalid networks response"),
    };
    let wallets = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        proofship_client::Command::Web3Wallets,
    )? {
        proofship_client::ResponsePayload::Web3Wallets { wallets } => wallets,
        _ => anyhow::bail!("invalid wallets response"),
    };
    let okx = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        proofship_client::Command::Web3OkxStatus,
    )? {
        proofship_client::ResponsePayload::Web3OkxStatus { status } => status,
        _ => anyhow::bail!("invalid okx response"),
    };
    let prefs = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        proofship_client::Command::Web3Prefs,
    )? {
        proofship_client::ResponsePayload::Web3Prefs { prefs } => prefs,
        _ => anyhow::bail!("invalid prefs response"),
    };
    Ok((networks, wallets, okx, prefs))
}

fn settings_card(theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(20.0))
        .py(px(14.0))
        .rounded(px(13.0))
        .bg(theme.raised)
}

fn settings_title(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text.into())
}

fn settings_hint(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(4.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .text_color(theme.text_tertiary)
        .child(text.into())
}

fn settings_error(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(theme.danger)
        .child(text.into())
}

fn settings_button(
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
        .cursor_default()
        .text_size(px(12.0))
        .text_color(theme.text)
        .hover(|style| style.bg(theme.overlay))
        .child(label.into())
}

fn wallet_dialog_copy(kind: WalletDialogKind) -> (String, String) {
    match kind {
        WalletDialogKind::Create => (
            tr!("web3.create_wallet_title"),
            tr!("web3.create_wallet_hint"),
        ),
        WalletDialogKind::Import => (
            tr!("web3.import_wallet_title"),
            tr!("web3.import_wallet_hint"),
        ),
        WalletDialogKind::Watch => (
            tr!("web3.watch_wallet_title"),
            tr!("web3.watch_wallet_hint"),
        ),
        WalletDialogKind::DevEnvKey => (tr!("web3.env_wallet_title"), tr!("web3.env_wallet_hint")),
    }
}

fn form_field(id: impl Into<ElementId>, input: Entity<TextInput>) -> Div {
    div().mt(px(8.0)).child(TextField::new(id, input).w_full())
}

fn render_toggle(
    theme: &Theme,
    id: impl Into<ElementId>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_color(theme.accent))
        .w(px(36.0))
        .h(px(20.0))
        .p(px(2.0))
        .flex_none()
        .rounded_full()
        .cursor_default()
        .bg(if enabled { theme.inverse } else { theme.inset })
        .border_1()
        .border_color(if enabled {
            theme.inverse
        } else {
            theme.border_strong
        })
        .flex()
        .items_center()
        .when(enabled, |element| element.justify_end())
        .child(div().w(px(14.0)).h(px(14.0)).rounded_full().bg(if enabled {
            theme.on_inverse
        } else {
            theme.text_tertiary
        }))
        .on_click(on_click)
}

fn slug(name: &str) -> String {
    let mut slug = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches('-').to_string()
}
