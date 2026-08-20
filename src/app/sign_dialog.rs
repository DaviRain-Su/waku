//! Preview-injected wallet: connect + confirm `eth_sendTransaction`.

use gpui::{KeyBinding, actions};
use serde_json::{Value, json};

use super::*;
use crate::browser::{BrowserEthEvent, BrowserView};
use waku_client::web3::{EvmNetwork, WalletAccount, WalletSource};

actions!(waku_sign_dialog, [DismissSignDialog]);

const DIALOG_CONTEXT: &str = "SignDialog";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        DismissSignDialog,
        Some(DIALOG_CONTEXT),
    )]);
}

pub(super) struct SignTxDialogState {
    request_id: u64,
    browser: Entity<BrowserView>,
    wallet: WalletAccount,
    network: EvmNetwork,
    to: Option<String>,
    data: String,
    sending: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl Waku {
    pub(super) fn handle_preview_eth(
        &mut self,
        browser: Entity<BrowserView>,
        event: &BrowserEthEvent,
        cx: &mut Context<Self>,
    ) {
        let id = event.id;
        match event.method.as_str() {
            "eth_chainId" => {
                let hex = format!("0x{:x}", self.preview_chain_id());
                browser.update(cx, |view, _| view.complete_eth(id, Ok(json!(hex))));
            }
            "net_version" => {
                let id_str = self.preview_chain_id().to_string();
                browser.update(cx, |view, _| view.complete_eth(id, Ok(json!(id_str))));
            }
            "eth_accounts" => {
                let accounts = self.connected_accounts();
                browser.update(cx, |view, _| view.complete_eth(id, Ok(json!(accounts))));
            }
            "eth_requestAccounts" => self.preview_request_accounts(browser, id, cx),
            "eth_sendTransaction" => {
                self.preview_send_transaction(browser, id, &event.params, cx)
            }
            "eth_getBalance"
            | "eth_call"
            | "eth_estimateGas"
            | "eth_gasPrice"
            | "eth_maxPriorityFeePerGas"
            | "eth_feeHistory"
            | "eth_getCode"
            | "eth_getStorageAt"
            | "eth_getTransactionCount"
            | "eth_getTransactionByHash"
            | "eth_getTransactionReceipt"
            | "eth_getBlockByNumber"
            | "eth_getBlockByHash"
            | "eth_blockNumber"
            | "eth_getLogs" => self.preview_rpc(browser, id, &event.method, &event.params, cx),
            "wallet_switchEthereumChain" | "wallet_addEthereumChain" => {
                self.preview_switch_chain(browser, id, &event.params, cx)
            }
            "wallet_requestPermissions" | "wallet_getPermissions" => {
                browser.update(cx, |view, _| {
                    view.complete_eth(id, Ok(json!([{ "parentCapability": "eth_accounts" }])))
                });
            }
            "personal_sign" | "eth_sign" | "eth_signTypedData" | "eth_signTypedData_v4" => {
                browser.update(cx, |view, _| {
                    view.complete_eth(id, Err((4200, tr!("web3.sign_message_unsupported"))))
                });
            }
            other => {
                browser.update(cx, |view, _| {
                    view.complete_eth(
                        id,
                        Err((4200, format!("Waku wallet does not implement {other}"))),
                    )
                });
            }
        }
    }

    fn preview_chain_id(&self) -> u64 {
        self.preview_network()
            .map(|network| network.chain_id)
            .unwrap_or(196)
    }

    fn preview_network(&self) -> Option<EvmNetwork> {
        self.selected_network().or_else(|| {
            self.web3_networks
                .as_ref()?
                .iter()
                .find(|network| network.enabled)
                .cloned()
        })
    }

    fn preview_signer(&self) -> Option<WalletAccount> {
        self.web3_wallets.as_ref()?.iter().find(is_signer).cloned()
    }

    fn connected_accounts(&self) -> Vec<String> {
        if !self.preview_connected {
            return Vec::new();
        }
        self.preview_signer()
            .map(|wallet| vec![wallet.address])
            .unwrap_or_default()
    }

    fn preview_request_accounts(
        &mut self,
        browser: Entity<BrowserView>,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        if self.web3_wallets.is_some() && self.web3_networks.is_some() {
            self.finish_request_accounts(browser, id, cx);
            return;
        }
        self.load_web3_then(cx, move |this, cx| {
            this.finish_request_accounts(browser, id, cx);
        });
    }

    fn preview_rpc(
        &mut self,
        browser: Entity<BrowserView>,
        id: u64,
        method: &str,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        if self.web3_networks.is_none() {
            let method = method.to_string();
            let params = params.clone();
            self.load_web3_then(cx, move |this, cx| {
                this.preview_rpc(browser, id, &method, &params, cx);
            });
            return;
        }
        let Some(network_id) = self.preview_network().map(|network| network.id) else {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((4902, tr!("web3.deploy_no_network"))))
            });
            return;
        };
        let method = method.to_string();
        let params = params.clone();
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        waku_client::Command::Web3Rpc {
                            network_id,
                            method,
                            params,
                        },
                    )? {
                        waku_client::ResponsePayload::Web3Rpc { result } => Ok(result),
                        _ => anyhow::bail!("invalid rpc response"),
                    }
                })
                .await;
            let _ = this.update(cx, |_, cx| {
                browser.update(cx, |view, _| match result {
                    Ok(value) => view.complete_eth(id, Ok(value)),
                    Err(error) => view.complete_eth(id, Err((-32603, error.to_string()))),
                });
            });
        })
        .detach();
    }

    fn finish_request_accounts(
        &mut self,
        browser: Entity<BrowserView>,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(wallet) = self.preview_signer() else {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((4100, tr!("web3.preview_no_wallet"))))
            });
            return;
        };
        self.preview_connected = true;
        browser.update(cx, |view, _| {
            view.complete_eth(id, Ok(json!([wallet.address])))
        });
        self.show_toast(tr!("web3.preview_connected"));
        cx.notify();
    }

    fn preview_switch_chain(
        &mut self,
        browser: Entity<BrowserView>,
        id: u64,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        let Some(chain_id) = first_object(params).and_then(|object| {
            object.get("chainId").and_then(parse_hex_u64)
        }) else {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((-32602, "missing chainId".into())))
            });
            return;
        };
        if self.web3_networks.is_none() {
            let params = params.clone();
            self.load_web3_then(cx, move |this, cx| {
                this.preview_switch_chain(browser, id, &params, cx);
            });
            return;
        }
        let Some(network) = self.web3_networks.as_ref().and_then(|networks| {
            networks
                .iter()
                .find(|network| network.enabled && network.chain_id == chain_id)
                .cloned()
        }) else {
            browser.update(cx, |view, _| {
                view.complete_eth(
                    id,
                    Err((
                        4902,
                        format!("unrecognized chain {chain_id} — add it in Settings → Networks"),
                    )),
                )
            });
            return;
        };
        self.set_selected_network(network.id, cx);
        browser.update(cx, |view, _| view.complete_eth(id, Ok(Value::Null)));
        cx.notify();
    }

    fn preview_send_transaction(
        &mut self,
        browser: Entity<BrowserView>,
        id: u64,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        if self.sign_dialog.is_some() {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((-32002, tr!("web3.sign_busy"))))
            });
            return;
        }
        if self.web3_wallets.is_none() || self.web3_networks.is_none() {
            let params = params.clone();
            self.load_web3_then(cx, move |this, cx| {
                this.preview_send_transaction(browser, id, &params, cx);
            });
            return;
        }
        let Some(wallet) = self.preview_signer() else {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((4100, tr!("web3.preview_no_wallet"))))
            });
            return;
        };
        let Some(network) = self.preview_network() else {
            browser.update(cx, |view, _| {
                view.complete_eth(id, Err((4902, tr!("web3.deploy_no_network"))))
            });
            return;
        };
        let tx = first_object(params);
        let to = tx
            .and_then(|object| object.get("to"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let data = tx
            .and_then(|object| object.get("data"))
            .and_then(Value::as_str)
            .unwrap_or("0x")
            .to_string();
        self.preview_connected = true;
        self.sign_dialog = Some(SignTxDialogState {
            request_id: id,
            browser,
            wallet,
            network,
            to,
            data,
            sending: false,
            error: None,
            focus: cx.focus_handle(),
        });
        self.show_toast(tr!("web3.sign_title"));
        cx.notify();
    }

    fn load_web3_then(
        &mut self,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let wallets = match daemon.request(
                Uuid::nil(),
                Uuid::nil(),
                waku_client::Command::Web3Wallets,
            ) {
                Ok(waku_client::ResponsePayload::Web3Wallets { wallets }) => wallets,
                Ok(_) => return,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.show_toast(error.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            let networks = match daemon.request(
                Uuid::nil(),
                Uuid::nil(),
                waku_client::Command::Web3Networks,
            ) {
                Ok(waku_client::ResponsePayload::Web3Networks { networks }) => networks,
                Ok(_) => return,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.show_toast(error.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.web3_wallets = Some(wallets);
                this.web3_networks = Some(networks);
                then(this, cx);
            });
        })
        .detach();
    }

    fn reject_sign_dialog(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.sign_dialog.take() {
            dialog.browser.update(cx, |view, _| {
                view.complete_eth(dialog.request_id, Err((4001, tr!("web3.sign_rejected"))))
            });
        }
        cx.notify();
    }

    fn confirm_sign_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.sign_dialog.as_mut() else {
            return;
        };
        if dialog.sending {
            return;
        }
        dialog.sending = true;
        dialog.error = None;
        let network_id = dialog.network.id.clone();
        let wallet_id = dialog.wallet.id.clone();
        let to = dialog.to.clone();
        let data = dialog.data.clone();
        let request_id = dialog.request_id;
        let browser = dialog.browser.clone();
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        waku_client::Command::Web3SendTx {
                            network_id,
                            wallet_id,
                            to,
                            data,
                        },
                    )? {
                        waku_client::ResponsePayload::Web3SendTx { tx_hash } => Ok(tx_hash),
                        _ => anyhow::bail!("invalid send response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(tx_hash) => {
                        if let Some(dialog) = this.sign_dialog.take() {
                            dialog.browser.update(cx, |view, _| {
                                view.complete_eth(request_id, Ok(json!(tx_hash)))
                            });
                        } else {
                            browser.update(cx, |view, _| {
                                view.complete_eth(request_id, Ok(json!(tx_hash)))
                            });
                        }
                        this.show_toast(tr!("web3.sign_sent"));
                    }
                    Err(error) => {
                        if let Some(dialog) = this.sign_dialog.as_mut() {
                            dialog.sending = false;
                            dialog.error = Some(error.to_string());
                        } else {
                            browser.update(cx, |view, _| {
                                view.complete_eth(request_id, Err((-32603, error.to_string())))
                            });
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_sign_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.sign_dialog.as_ref()?;
        let theme = Theme::current(cx);
        let sending = dialog.sending;
        let error = dialog.error.clone();
        let focus = dialog.focus.clone();
        let wallet_line = format!("{} · {}", dialog.wallet.label, dialog.wallet.address);
        let network_line = format!("{} ({})", dialog.network.name, dialog.network.chain_id);
        let to_line = dialog
            .to
            .clone()
            .unwrap_or_else(|| tr!("web3.sign_create"));
        let data_line = truncate_hex(&dialog.data);

        let mut body = div()
            .id("sign-dialog")
            .track_focus(&focus)
            .key_context(DIALOG_CONTEXT)
            .on_action(cx.listener(|this, _: &DismissSignDialog, _, cx| {
                this.reject_sign_dialog(cx)
            }))
            .w(px(440.0))
            .p(px(20.0))
            .rounded(px(16.0))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("web3.sign_title")),
            )
            .child(hint_line(&theme, tr!("web3.sign_description")))
            .child(hint_line(&theme, format!("{} · {}", tr!("web3.network"), network_line)))
            .child(hint_line(&theme, format!("{} · {}", tr!("web3.wallet"), wallet_line)))
            .child(hint_line(&theme, format!("{} · {}", tr!("web3.sign_to"), to_line)))
            .child(hint_line(&theme, format!("{} · {}", tr!("web3.sign_data"), data_line)));
        if let Some(error) = error {
            body = body.child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.danger)
                    .child(SharedString::from(error)),
            );
        }
        body = body.child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(
                    div()
                        .id("sign-reject")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(theme.accent))
                        .px(px(12.0))
                        .h(px(30.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .cursor_default()
                        .text_color(theme.text_secondary)
                        .child(tr!("web3.sign_reject"))
                        .on_click(cx.listener(|this, _, _, cx| this.reject_sign_dialog(cx))),
                )
                .child(
                    div()
                        .id("sign-confirm")
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
                        .child(if sending {
                            SharedString::from(tr!("web3.signing"))
                        } else {
                            SharedString::from(tr!("web3.sign_confirm"))
                        })
                        .when(!sending, |button| {
                            button.on_click(
                                cx.listener(|this, _, _, cx| this.confirm_sign_dialog(cx)),
                            )
                        }),
                ),
        );

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("sign-dialog-layer")
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
                cx.listener(|this, _, _, cx| this.reject_sign_dialog(cx)),
            )
            .child(body);
        Some(gpui::deferred(layer).with_priority(5).into_any_element())
    }
}

fn is_signer(wallet: &&WalletAccount) -> bool {
    matches!(wallet.source, WalletSource::Local | WalletSource::DevEnvKey)
        && !wallet.address.trim().is_empty()
}

fn first_object(params: &Value) -> Option<&serde_json::Map<String, Value>> {
    params.as_array()?.first()?.as_object()
}

fn parse_hex_u64(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    let text = value.as_str()?.trim();
    if let Some(hex) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

fn truncate_hex(data: &str) -> String {
    if data.len() <= 18 {
        return data.to_string();
    }
    format!("{}…{}", &data[..10], &data[data.len() - 6..])
}

fn hint_line(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(theme.text_tertiary)
        .child(text.into())
}
