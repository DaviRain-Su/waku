//! Detect a local frontend and serve it on loopback. No render-path I/O.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use proofship_protocol::ship::{FrontendDetect, PreviewStatus};

struct ActivePreview {
    url: String,
    detect: FrontendDetect,
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    child: Option<Child>,
}

#[derive(Default)]
struct PreviewHub {
    inner: Mutex<HashMap<String, ActivePreview>>,
}

fn hub() -> &'static PreviewHub {
    static HUB: OnceLock<PreviewHub> = OnceLock::new();
    HUB.get_or_init(PreviewHub::default)
}

fn cwd_key(cwd: &Path) -> String {
    cwd.canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub fn detect_frontend(cwd: &Path) -> FrontendDetect {
    let wrangler = wrangler_config(cwd).is_some();
    let vercel = cwd.join("vercel.json").is_file()
        || read_package_json(cwd).is_some_and(|package| package.contains("\"vercel\""));
    let project_name = wrangler_name(cwd).unwrap_or_else(|| slug_from_path(cwd));
    let package = read_package_json(cwd);
    let dist = first_existing(&[
        cwd.join("dist/index.html"),
        cwd.join("build/index.html"),
        cwd.join("out/index.html"),
        cwd.join("public/index.html"),
        cwd.join("index.html"),
    ]);
    let vite = package.as_ref().is_some_and(|body| looks_like_vite(body));
    let next = package.as_ref().is_some_and(|body| looks_like_next(body));
    let spa = dist
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .as_deref()
        .map(html_is_spa)
        .unwrap_or(vite || next);

    if next {
        return FrontendDetect {
            kind: "next".into(),
            root: cwd.display().to_string(),
            hint: "Next.js app — Preview starts the local dev server".into(),
            spa: true,
            wrangler,
            vercel,
            project_name,
        };
    }
    if vite {
        return FrontendDetect {
            kind: "vite".into(),
            root: cwd.display().to_string(),
            hint: "Vite app — Preview starts the local dev server".into(),
            spa,
            wrangler,
            vercel,
            project_name,
        };
    }
    if wrangler && dist.is_none() && package.is_none() {
        return FrontendDetect {
            kind: "worker".into(),
            root: cwd.display().to_string(),
            hint: "Wrangler Worker — no static UI; use Ship to publish".into(),
            spa: false,
            wrangler: true,
            vercel,
            project_name,
        };
    }
    if let Some(index) = dist {
        let root = index
            .parent()
            .unwrap_or(cwd)
            .to_path_buf()
            .display()
            .to_string();
        return FrontendDetect {
            kind: "static".into(),
            root,
            hint: "Static HTML — Preview serves it on loopback".into(),
            spa,
            wrangler,
            vercel,
            project_name,
        };
    }
    FrontendDetect {
        kind: "none".into(),
        root: cwd.display().to_string(),
        hint: "No frontend found — add index.html, dist/, or a Vite/Next app".into(),
        spa: false,
        wrangler,
        vercel,
        project_name,
    }
}

pub fn scan(cwd: &Path) -> FrontendDetect {
    detect_frontend(cwd)
}

pub fn status(cwd: &Path) -> PreviewStatus {
    let detect = detect_frontend(cwd);
    let key = cwd_key(cwd);
    let guard = hub()
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(active) = guard.get(&key) {
        return PreviewStatus {
            detect: active.detect.clone(),
            running: true,
            url: Some(active.url.clone()),
            error: None,
        };
    }
    PreviewStatus {
        detect,
        running: false,
        url: None,
        error: None,
    }
}

pub fn start(cwd: &Path) -> anyhow::Result<PreviewStatus> {
    if !cwd.is_dir() {
        bail!("cwd is not a directory: {}", cwd.display());
    }
    let key = cwd_key(cwd);
    {
        let guard = hub()
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(active) = guard.get(&key) {
            return Ok(PreviewStatus {
                detect: active.detect.clone(),
                running: true,
                url: Some(active.url.clone()),
                error: None,
            });
        }
    }
    let detect = detect_frontend(cwd);
    if detect.kind == "none" || detect.kind == "worker" {
        return Ok(PreviewStatus {
            detect,
            running: false,
            url: None,
            error: None,
        });
    }
    if let Some(url) = reuse_listening_url(cwd) {
        let status = PreviewStatus {
            detect: detect.clone(),
            running: true,
            url: Some(url.clone()),
            error: None,
        };
        hub()
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                key,
                ActivePreview {
                    url,
                    detect,
                    shutdown: None,
                    child: None,
                },
            );
        return Ok(status);
    }

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}/");
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    let mut child = None;
    if matches!(detect.kind.as_str(), "vite" | "next") && npx_available() {
        drop(listener);
        child = Some(spawn_dev(cwd, port, &detect.kind)?);
        wait_http_ok(&url, Duration::from_secs(25));
    } else {
        let root = PathBuf::from(&detect.root);
        thread::Builder::new()
            .name("waku-preview".into())
            .spawn(move || serve_static(listener, root, shutdown_rx))
            .map_err(|error| anyhow!("could not start preview: {error}"))?;
    }
    let status = PreviewStatus {
        detect: detect.clone(),
        running: true,
        url: Some(url.clone()),
        error: None,
    };
    hub()
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            key,
            ActivePreview {
                url,
                detect,
                shutdown: child.is_none().then_some(shutdown_tx),
                child,
            },
        );
    Ok(status)
}

pub fn stop(cwd: &Path) -> PreviewStatus {
    let key = cwd_key(cwd);
    let mut guard = hub()
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(mut active) = guard.remove(&key) {
        if let Some(tx) = active.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(mut child) = active.child.take() {
            let _ = child.kill();
        }
    }
    PreviewStatus {
        detect: detect_frontend(cwd),
        running: false,
        url: None,
        error: None,
    }
}

fn reuse_listening_url(cwd: &Path) -> Option<String> {
    let log = cwd.join(".waku-preview-port");
    if let Ok(raw) = std::fs::read_to_string(log) {
        let port = raw.trim().parse::<u16>().ok()?;
        let url = format!("http://127.0.0.1:{port}/");
        if wait_http_ok(&url, Duration::from_millis(200)) {
            return Some(url);
        }
    }
    None
}

fn spawn_dev(cwd: &Path, port: u16, kind: &str) -> anyhow::Result<Child> {
    let mut command = Command::new(npx_bin());
    command
        .current_dir(cwd)
        .env("NO_UPDATE_NOTIFIER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match kind {
        "next" => {
            command.args([
                "--yes",
                "next",
                "dev",
                "-p",
                &port.to_string(),
                "-H",
                "127.0.0.1",
            ]);
        }
        _ => {
            command.args([
                "--yes",
                "vite",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--strictPort",
            ]);
        }
    }
    command
        .spawn()
        .map_err(|error| anyhow!("could not start {kind} preview: {error}"))
}

fn serve_static(listener: TcpListener, root: PathBuf, shutdown: std::sync::mpsc::Receiver<()>) {
    listener.set_nonblocking(true).ok();
    loop {
        if shutdown.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 2048];
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let relative = path.trim_start_matches('/').split('?').next().unwrap_or("");
                let mut file = root.join(relative);
                if relative.is_empty() || file.is_dir() {
                    file = root.join("index.html");
                }
                if !file.starts_with(&root) {
                    let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n");
                    continue;
                }
                match std::fs::read(&file) {
                    Ok(bytes) => write_static(&mut stream, &file, &bytes),
                    Err(_) => {
                        let fallback = root.join("index.html");
                        if let Ok(bytes) = std::fs::read(&fallback) {
                            write_static(&mut stream, &fallback, &bytes);
                        } else {
                            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n");
                        }
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn wait_http_ok(url: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(response) = ureq_get(url) {
            if response {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(150));
    }
    false
}

fn ureq_get(url: &str) -> anyhow::Result<bool> {
    let parsed = url.trim_start_matches("http://").trim_end_matches('/');
    let (host, port) = parsed.split_once(':').unwrap_or((parsed, "80"));
    let mut stream = std::net::TcpStream::connect_timeout(
        &format!("{host}:{port}").parse()?,
        Duration::from_millis(200),
    )?;
    stream.write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")?;
    let mut buf = [0u8; 16];
    let n = stream.read(&mut buf)?;
    Ok(n > 0)
}

fn npx_available() -> bool {
    Command::new(npx_bin())
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn npx_bin() -> &'static str {
    if cfg!(windows) { "npx.cmd" } else { "npx" }
}

fn write_static(stream: &mut std::net::TcpStream, path: &Path, bytes: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        mime_for(path),
        bytes.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(bytes);
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("map") => "application/json",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|path| path.is_file()).cloned()
}

fn read_package_json(cwd: &Path) -> Option<String> {
    std::fs::read_to_string(cwd.join("package.json")).ok()
}

fn looks_like_vite(package: &str) -> bool {
    package.contains("\"vite\"") || package.contains("vite@")
}

fn looks_like_next(package: &str) -> bool {
    package.contains("\"next\"") || package.contains("next@")
}

fn html_is_spa(html: &str) -> bool {
    html.contains("type=\"module\"") || html.contains("id=\"root\"") || html.contains("id='root'")
}

fn wrangler_config(cwd: &Path) -> Option<PathBuf> {
    ["wrangler.toml", "wrangler.json", "wrangler.jsonc"]
        .into_iter()
        .map(|name| cwd.join(name))
        .find(|path| path.is_file())
}

fn wrangler_name(cwd: &Path) -> Option<String> {
    let path = wrangler_config(cwd)?;
    let raw = std::fs::read_to_string(path).ok()?;
    raw.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("name")
            .and_then(|rest| rest.trim().strip_prefix('='))
            .map(|value| value.trim().trim_matches(['"', '\'']).to_string())
            .filter(|value| !value.is_empty())
    })
}

fn slug_from_path(cwd: &Path) -> String {
    cwd.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("app")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_static_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>hi</h1>").unwrap();
        let detect = detect_frontend(dir.path());
        assert_eq!(detect.kind, "static");
    }

    #[test]
    fn detects_vite_from_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vite":"6.0.0"}}"#,
        )
        .unwrap();
        assert_eq!(detect_frontend(dir.path()).kind, "vite");
    }

    #[test]
    fn none_without_frontend() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_frontend(dir.path()).kind, "none");
    }

    #[test]
    fn js_is_served_as_javascript() {
        assert_eq!(
            mime_for(Path::new("vendor/ethers.umd.min.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            mime_for(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
    }
}
