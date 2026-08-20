//! Local frontend preview: scan off-thread, then open the existing Browser.

use super::*;
use proofship_client::ship::PreviewStatus;

impl Waku {
    pub(super) fn session_cwd(&self) -> Option<PathBuf> {
        let session = self.selected_session()?;
        self.workspace_path_for_session(session)
            .map(Path::to_path_buf)
            .or_else(|| {
                self.state
                    .projects
                    .iter()
                    .find(|project| project.id == session.project_id)
                    .map(|project| project.path.clone())
            })
    }

    pub(super) fn preview_available(&self) -> bool {
        self.preview_detect
            .as_ref()
            .is_some_and(|detect| detect.kind != "none")
    }

    pub(super) fn refresh_preview_detect(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.session_cwd() else {
            self.preview_detect = None;
            self.preview_status = None;
            return;
        };
        self.preview_generation += 1;
        let generation = self.preview_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let detect = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::PreviewScan { cwd },
                    ) {
                        Ok(proofship_client::ResponsePayload::PreviewScan { detect }) => {
                            Some(detect)
                        }
                        _ => None,
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.preview_generation != generation {
                    return;
                }
                this.preview_detect = detect;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_local_preview(&mut self, cx: &mut Context<Self>) {
        let Some(cwd) = self.session_cwd() else {
            self.show_toast(tr!("preview.no_workspace"));
            return;
        };
        if self
            .preview_detect
            .as_ref()
            .is_some_and(|detect| detect.kind == "none" || detect.kind == "worker")
        {
            self.show_toast(
                self.preview_detect
                    .as_ref()
                    .map(|detect| detect.hint.clone())
                    .unwrap_or_else(|| tr!("preview.none")),
            );
            return;
        }
        self.preview_generation += 1;
        let generation = self.preview_generation;
        let daemon = self.daemon.client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match daemon.request(
                        Uuid::nil(),
                        Uuid::nil(),
                        proofship_client::Command::PreviewStart { cwd },
                    )? {
                        proofship_client::ResponsePayload::PreviewStatus { status } => Ok(status),
                        _ => anyhow::bail!("invalid preview response"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.preview_generation != generation {
                    return;
                }
                match result {
                    Ok(status) => this.open_preview_url(status, cx),
                    Err(error) => this.show_toast(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_preview_url(&mut self, status: PreviewStatus, cx: &mut Context<Self>) {
        self.preview_detect = Some(status.detect.clone());
        self.preview_status = Some(status.clone());
        let Some(url) = status.url.filter(|_| status.running) else {
            self.show_toast(
                status
                    .error
                    .or(Some(status.detect.hint))
                    .unwrap_or_else(|| tr!("preview.none")),
            );
            return;
        };
        // WKWebView has no Chrome/OKX/MetaMask extensions. Keep the in-app
        // preview, and also open the same loopback URL in the system browser
        // so a dApp that needs `window.ethereum` can actually connect.
        cx.open_url(&url);
        self.show_toast(tr!("preview.opened_externally"));
        self.open_browser_url(url, cx);
    }

    pub(super) fn open_browser_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.pending_preview_url = Some(url);
        if let Some(index) = self
            .right_panel_surfaces
            .iter()
            .position(|surface| matches!(surface, RightPanelSurface::Browser(_)))
        {
            self.right_panel_active_surface = Some(index);
            self.reveal_right_panel_tab(index);
            self.request_active_browser_focus();
            self.set_right_panel_visible(true, cx);
        } else {
            self.open_right_panel_surface(RightPanelSurface::new_browser(), cx);
        }
        self.apply_pending_preview_url(cx);
    }

    pub(super) fn apply_pending_preview_url(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.pending_preview_url.clone() else {
            return;
        };
        let Some(browser_id) = self
            .active_right_panel_surface()
            .and_then(RightPanelSurface::browser_id)
        else {
            return;
        };
        let Some(browser) = self.right_panel_browsers.get(&browser_id).cloned() else {
            return;
        };
        self.pending_preview_url = None;
        browser.update(cx, |view, cx| view.navigate_to_url(url, cx));
    }

    pub(super) fn stop_preview_for_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        let daemon = self.daemon.client();
        cx.background_executor()
            .spawn(async move {
                let _ = daemon.request(
                    Uuid::nil(),
                    Uuid::nil(),
                    proofship_client::Command::PreviewStop { cwd },
                );
            })
            .detach();
    }
}
