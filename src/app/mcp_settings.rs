//! Settings → MCP. Daemon owns the catalog and CLI tokens; frames read cache.

use super::*;
use proofship_client::ship::{HostingProvider, HostingTokenStatus, McpServer, McpTransport};

pub(super) struct McpDialog {
    name: Entity<TextInput>,
    endpoint: Entity<TextInput>,
    args: Entity<TextInput>,
    transport: McpTransport,
}

impl Waku {
    pub(super) fn ensure_mcp_settings(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.mcp_pending && !force {
            return;
        }
        if !force && self.mcp_servers.is_some() && self.mcp_tokens.is_some() {
            return;
        }
        self.mcp_pending = true;
        self.mcp_generation += 1;
        let generation = self.mcp_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_executor()
                .spawn(async move { load_mcp_snapshot(&daemon) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.mcp_generation != generation {
                    return;
                }
                this.mcp_pending = false;
                match snapshot {
                    Ok((servers, tokens)) => {
                        this.mcp_servers = Some(servers);
                        this.mcp_tokens = Some(tokens);
                        this.mcp_error = None;
                    }
                    Err(error) => this.mcp_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_mcp_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let mut column = div().mt(px(15.0)).flex().flex_col().gap(px(12.0));
        if let Some(error) = &self.mcp_error {
            column = column.child(mcp_error(&theme, error));
        }
        column = column
            .child(
                mcp_card(&theme)
                    .child(mcp_title(&theme, tr!("mcp.description_title")))
                    .child(mcp_hint(&theme, tr!("mcp.description"))),
            )
            .child(self.render_hosting_token_card(HostingProvider::Cloudflare, &theme, cx))
            .child(self.render_hosting_token_card(HostingProvider::Vercel, &theme, cx));
        if let Some(servers) = &self.mcp_servers {
            for (index, server) in servers.iter().enumerate() {
                column = column.child(self.render_mcp_row(index, server, &theme, cx));
            }
        } else {
            column = column.child(mcp_hint(&theme, tr!("mcp.loading")));
        }
        column = column.child(div().flex().flex_wrap().gap(px(8.0)).children([
            mcp_button(&theme, "mcp-add-http", tr!("mcp.add_http")).on_click(cx.listener(
                |this, _, window, cx| this.open_mcp_dialog(McpTransport::Http, window, cx),
            )),
            mcp_button(&theme, "mcp-add-stdio", tr!("mcp.add_stdio")).on_click(cx.listener(
                |this, _, window, cx| this.open_mcp_dialog(McpTransport::Stdio, window, cx),
            )),
        ]));
        if let Some(dialog) = &self.mcp_dialog {
            column = column.child(self.render_mcp_dialog(dialog, &theme, cx));
        }
        if self.mcp_github_token_for.is_some() {
            column = column.child(self.render_github_token_dialog(&theme, cx));
        }
        column.into_any_element()
    }

    fn render_hosting_token_card(
        &self,
        provider: HostingProvider,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let status = self
            .mcp_tokens
            .as_ref()
            .and_then(|tokens| {
                tokens
                    .iter()
                    .find(|token| token.provider == provider.id())
                    .cloned()
            })
            .unwrap_or_default();
        let enabled = status.enabled;
        let input = match provider {
            HostingProvider::Cloudflare => self.mcp_cf_input.clone(),
            HostingProvider::Vercel => self.mcp_vercel_input.clone(),
        };
        let (title, description, field_id, save_id) = match provider {
            HostingProvider::Cloudflare => (
                tr!("mcp.cloudflare_token"),
                tr!("mcp.cloudflare_token_description"),
                "mcp-cf-token",
                "mcp-cf-save",
            ),
            HostingProvider::Vercel => (
                tr!("mcp.vercel_token"),
                tr!("mcp.vercel_token_description"),
                "mcp-vercel-token",
                "mcp-vercel-save",
            ),
        };
        mcp_card(theme)
            .child(mcp_title(theme, title))
            .child(mcp_hint(theme, description))
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(TextField::new(field_id, input).flex_1().max_w(px(360.0)))
                    .child(mcp_button(theme, save_id, tr!("mcp.save_token")).on_click(
                        cx.listener(move |this, _, _, cx| this.save_hosting_token(provider, cx)),
                    )),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(mcp_hint(
                        theme,
                        status
                            .key_hint
                            .clone()
                            .unwrap_or_else(|| tr!("mcp.token_not_configured")),
                    ))
                    .child(render_mcp_toggle(
                        theme,
                        format!("mcp-token-enabled-{}", provider.id()),
                        enabled,
                        cx.listener(move |this, _, _, cx| {
                            this.set_hosting_token_enabled(provider, !enabled, cx)
                        }),
                    )),
            )
    }

    fn render_mcp_row(
        &self,
        index: usize,
        server: &McpServer,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let enabled = server.enabled;
        let id = server.id.clone();
        let builtin = server.builtin;
        let web3_attached = server
            .source
            .as_deref()
            .is_some_and(|source| source.starts_with("web3-"));
        let detail = match server.transport {
            McpTransport::Http => server.url.clone().unwrap_or_default(),
            McpTransport::Stdio => {
                let command = server.command.clone().unwrap_or_default();
                if server.args.is_empty() {
                    command
                } else {
                    format!("{command} {}", server.args.join(" "))
                }
            }
        };
        let source = match server.source.as_deref() {
            Some("web3-okx") => tr!("mcp.source_okx"),
            Some("web3-pf") => tr!("mcp.source_pf"),
            _ if builtin => tr!("mcp.builtin"),
            _ => tr!("mcp.custom"),
        };
        mcp_card(theme).child(
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
                        .child(mcp_title(theme, server.name.clone()))
                        .child(mcp_hint(
                            theme,
                            format!("{} · {} · {}", source, mcp_auth_label(server), detail),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .when(!web3_attached && server.auth == "needed", |row| {
                            let id = server.id.clone();
                            row.child(
                                mcp_button(theme, ("mcp-authorize", index), tr!("mcp.authorize"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.authorize_mcp_server(&id, cx)
                                    })),
                            )
                        })
                        .when(!web3_attached && server.auth == "authorized", |row| {
                            let id = server.id.clone();
                            row.child(
                                mcp_button(theme, ("mcp-disconnect", index), tr!("mcp.disconnect"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.disconnect_mcp_server(&id, cx)
                                    })),
                            )
                        })
                        .when(!builtin && !web3_attached, |row| {
                            row.child(
                                mcp_button(theme, ("mcp-remove", index), tr!("mcp.remove"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_mcp_server(&id, cx)
                                    })),
                            )
                        })
                        .when(!web3_attached, |row| {
                            row.child(render_mcp_toggle(
                                theme,
                                ("mcp-enabled", index),
                                enabled,
                                cx.listener({
                                    let id = server.id.clone();
                                    move |this, _, _, cx| this.set_mcp_enabled(&id, !enabled, cx)
                                }),
                            ))
                        }),
                ),
        )
    }

    fn render_mcp_dialog(&self, dialog: &McpDialog, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let http = matches!(dialog.transport, McpTransport::Http);
        mcp_card(theme)
            .child(mcp_title(
                theme,
                if http {
                    tr!("mcp.add_http")
                } else {
                    tr!("mcp.add_stdio")
                },
            ))
            .child(mcp_hint(
                theme,
                if http {
                    tr!("mcp.add_http_hint")
                } else {
                    tr!("mcp.add_stdio_hint")
                },
            ))
            .child(
                div()
                    .mt(px(8.0))
                    .child(TextField::new("mcp-name", dialog.name.clone()).w_full()),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .child(TextField::new("mcp-endpoint", dialog.endpoint.clone()).w_full()),
            )
            .when(!http, |column| {
                column.child(
                    div()
                        .mt(px(8.0))
                        .child(TextField::new("mcp-args", dialog.args.clone()).w_full()),
                )
            })
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        mcp_button(theme, "mcp-dialog-cancel", tr!("common.cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.mcp_dialog = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        mcp_button(theme, "mcp-dialog-save", tr!("mcp.add"))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_mcp_dialog(cx))),
                    ),
            )
    }

    fn open_mcp_dialog(
        &mut self,
        transport: McpTransport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (endpoint_placeholder, args_placeholder) = match transport {
            McpTransport::Http => (tr!("mcp.url_placeholder"), String::new()),
            McpTransport::Stdio => (tr!("mcp.command_placeholder"), tr!("mcp.args_placeholder")),
        };
        self.mcp_dialog = Some(McpDialog {
            name: cx.new(|cx| TextInput::new(window, cx).placeholder(tr!("mcp.name_placeholder"))),
            endpoint: cx.new(|cx| TextInput::new(window, cx).placeholder(endpoint_placeholder)),
            args: cx.new(|cx| TextInput::new(window, cx).placeholder(args_placeholder)),
            transport,
        });
        cx.notify();
    }

    fn submit_mcp_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.mcp_dialog else {
            return;
        };
        let name = dialog.name.read(cx).content().trim().to_string();
        let endpoint = dialog.endpoint.read(cx).content().trim().to_string();
        let args = dialog
            .args
            .read(cx)
            .content()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let transport = dialog.transport;
        self.mcp_dialog = None;
        let server = match transport {
            McpTransport::Http => McpServer {
                id: String::new(),
                name,
                transport,
                url: Some(endpoint),
                command: None,
                args: Vec::new(),
                enabled: true,
                builtin: false,
                source: None,
                auth: String::new(),
                auth_account: None,
            },
            McpTransport::Stdio => McpServer {
                id: String::new(),
                name,
                transport,
                url: None,
                command: Some(endpoint),
                args,
                enabled: true,
                builtin: false,
                source: None,
                auth: String::new(),
                auth_account: None,
            },
        };
        self.mutate_mcp(
            proofship_client::Command::McpUpsert { server },
            |this, payload| {
                if let proofship_client::ResponsePayload::McpList { servers } = payload {
                    this.mcp_servers = Some(servers);
                }
            },
            cx,
        );
    }

    fn render_github_token_dialog(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        mcp_card(theme)
            .child(mcp_title(theme, tr!("mcp.github_token_title")))
            .child(mcp_hint(theme, tr!("mcp.github_token_hint")))
            .child(
                div().mt(px(8.0)).child(
                    TextField::new("mcp-github-token", self.mcp_github_input.clone()).w_full(),
                ),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        mcp_button(theme, "mcp-github-token-cancel", tr!("common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.mcp_github_token_for = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        mcp_button(theme, "mcp-github-token-save", tr!("mcp.github_token_save"))
                            .on_click(cx.listener(|this, _, _, cx| this.submit_github_token(cx))),
                    ),
            )
    }

    fn submit_github_token(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.mcp_github_token_for.clone() else {
            return;
        };
        let token = self.mcp_github_input.read(cx).content().trim().to_string();
        self.mcp_github_input
            .update(cx, |input, cx| input.set_content(String::new(), cx));
        if token.is_empty() {
            self.mcp_error = Some(tr!("mcp.github_token_required"));
            cx.notify();
            return;
        }
        self.mcp_pending = true;
        self.mcp_generation += 1;
        let generation = self.mcp_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::McpAuthorize {
                            id,
                            token: Some(token),
                        },
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.mcp_generation != generation {
                    return;
                }
                this.mcp_pending = false;
                match result {
                    Ok(proofship_client::ResponsePayload::McpAuthorize { servers, .. }) => {
                        this.mcp_servers = Some(servers);
                        this.mcp_github_token_for = None;
                        this.mcp_error = None;
                        this.show_toast(tr!("mcp.signed_in"));
                    }
                    Ok(_) => this.mcp_error = Some("invalid authorize response".into()),
                    Err(error) => this.mcp_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn authorize_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.mcp_pending = true;
        self.mcp_generation += 1;
        let generation = self.mcp_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let id = id.clone();
                    async move {
                        daemon.request(
                            Uuid::nil(),
                            Uuid::nil(),
                            proofship_client::Command::McpAuthorize { id, token: None },
                        )
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.mcp_generation != generation {
                    return;
                }
                this.mcp_pending = false;
                match result {
                    Ok(proofship_client::ResponsePayload::McpAuthorize { url, servers }) => {
                        this.mcp_servers = Some(servers.clone());
                        this.mcp_error = None;
                        let authorized = servers
                            .iter()
                            .any(|server| server.id == id && server.auth == "authorized");
                        if authorized {
                            this.mcp_github_token_for = None;
                            this.show_toast(tr!("mcp.signed_in"));
                        } else if let Some(url) = url {
                            cx.open_url(&url);
                            if url.contains("github.com/settings/tokens") {
                                this.mcp_github_token_for = Some(id);
                            } else {
                                this.poll_mcp_authorization(id, cx);
                            }
                        } else {
                            this.show_toast(tr!("mcp.public_no_login"));
                        }
                    }
                    Ok(_) => this.mcp_error = Some("invalid authorize response".into()),
                    Err(error) => this.mcp_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn poll_mcp_authorization(&mut self, id: String, cx: &mut Context<Self>) {
        let daemon = self.daemon.client();
        let generation = self.mcp_generation;
        cx.spawn(async move |this, cx| {
            for _ in 0..90 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let snapshot = cx
                    .background_executor()
                    .spawn({
                        let daemon = daemon.clone();
                        async move { load_mcp_snapshot(&daemon) }
                    })
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        if this.mcp_generation != generation {
                            return true;
                        }
                        if let Ok((servers, tokens)) = snapshot {
                            this.mcp_servers = Some(servers.clone());
                            this.mcp_tokens = Some(tokens);
                            this.mcp_error = None;
                            cx.notify();
                            return servers
                                .iter()
                                .any(|server| server.id == id && server.auth == "authorized");
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

    fn disconnect_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mutate_mcp(
            proofship_client::Command::McpDisconnect { id: id.to_string() },
            |this, payload| {
                if let proofship_client::ResponsePayload::McpList { servers } = payload {
                    this.mcp_servers = Some(servers);
                }
            },
            cx,
        );
    }

    fn set_mcp_enabled(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        self.mutate_mcp(
            proofship_client::Command::McpSetEnabled {
                id: id.to_string(),
                enabled,
            },
            |this, payload| {
                if let proofship_client::ResponsePayload::McpList { servers } = payload {
                    this.mcp_servers = Some(servers);
                }
            },
            cx,
        );
    }

    fn remove_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mutate_mcp(
            proofship_client::Command::McpRemove { id: id.to_string() },
            |this, payload| {
                if let proofship_client::ResponsePayload::McpList { servers } = payload {
                    this.mcp_servers = Some(servers);
                }
            },
            cx,
        );
    }

    fn save_hosting_token(&mut self, provider: HostingProvider, cx: &mut Context<Self>) {
        let token = match provider {
            HostingProvider::Cloudflare => self.mcp_cf_input.read(cx).content().trim().to_string(),
            HostingProvider::Vercel => self.mcp_vercel_input.read(cx).content().trim().to_string(),
        };
        let input = match provider {
            HostingProvider::Cloudflare => self.mcp_cf_input.clone(),
            HostingProvider::Vercel => self.mcp_vercel_input.clone(),
        };
        input.update(cx, |input, cx| input.set_content(String::new(), cx));
        self.mutate_mcp(
            proofship_client::Command::HostingSetToken {
                provider,
                api_token: Some(token),
                enabled: None,
            },
            |this, payload| {
                if let proofship_client::ResponsePayload::HostingTokens { tokens } = payload {
                    this.mcp_tokens = Some(tokens);
                }
            },
            cx,
        );
    }

    fn set_hosting_token_enabled(
        &mut self,
        provider: HostingProvider,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.mutate_mcp(
            proofship_client::Command::HostingSetToken {
                provider,
                api_token: None,
                enabled: Some(enabled),
            },
            |this, payload| {
                if let proofship_client::ResponsePayload::HostingTokens { tokens } = payload {
                    this.mcp_tokens = Some(tokens);
                }
            },
            cx,
        );
    }

    fn mutate_mcp(
        &mut self,
        command: proofship_client::Command,
        apply: impl FnOnce(&mut Self, proofship_client::ResponsePayload) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.mcp_pending = true;
        self.mcp_generation += 1;
        let generation = self.mcp_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon.request(Uuid::nil(), Uuid::nil(), command) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.mcp_generation != generation {
                    return;
                }
                this.mcp_pending = false;
                match result {
                    Ok(payload) => {
                        apply(this, payload);
                        this.mcp_error = None;
                    }
                    Err(error) => this.mcp_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn mcp_auth_label(server: &McpServer) -> String {
    match server.auth.as_str() {
        "authorized" => server
            .auth_account
            .clone()
            .unwrap_or_else(|| tr!("mcp.auth_authorized")),
        "public" => tr!("mcp.auth_public"),
        "needed" => tr!("mcp.auth_needed"),
        _ => tr!("mcp.auth_none"),
    }
}

fn load_mcp_snapshot(
    daemon: &proofship_client::DaemonClient,
) -> anyhow::Result<(Vec<McpServer>, Vec<HostingTokenStatus>)> {
    let servers =
        match daemon.request(Uuid::nil(), Uuid::nil(), proofship_client::Command::McpList)? {
            proofship_client::ResponsePayload::McpList { servers } => servers,
            _ => anyhow::bail!("invalid mcp list response"),
        };
    let tokens = match daemon.request(
        Uuid::nil(),
        Uuid::nil(),
        proofship_client::Command::HostingTokens,
    )? {
        proofship_client::ResponsePayload::HostingTokens { tokens } => tokens,
        _ => anyhow::bail!("invalid hosting tokens response"),
    };
    Ok((servers, tokens))
}

fn mcp_card(theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(20.0))
        .py(px(14.0))
        .rounded(px(13.0))
        .bg(theme.raised)
}

fn mcp_title(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(text.into())
}

fn mcp_hint(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(4.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .text_color(theme.text_tertiary)
        .child(text.into())
}

fn mcp_error(theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(theme.danger)
        .child(text.into())
}

fn mcp_button(
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

fn render_mcp_toggle(
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
