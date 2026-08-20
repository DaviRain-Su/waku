//! MCP OAuth for remote HTTP servers. Tokens stay in 0600 files.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail};
use base64::Engine as _;
use proofship_protocol::ship::{McpServer, McpTransport};
use sha2::{Digest, Sha256};

const FILE_MODE: u32 = 0o600;
const CALLBACK_HTML: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<!doctype html><meta charset=utf-8><title>ProofShip</title><p>Signed in. You can return to ProofShip.</p>";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenFile {
    #[serde(default)]
    client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(default)]
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_endpoint: Option<String>,
}

struct PendingAuth {
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    code_verifier: String,
    resource: String,
    state: String,
}

#[derive(Clone)]
pub struct OAuthStore {
    root: PathBuf,
}

static STORE: OnceLock<OAuthStore> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashMap<String, PendingAuth>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, PendingAuth>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

impl OAuthStore {
    pub fn new(data_dir: &Path) -> Self {
        let store = Self {
            root: data_dir.join("mcp-oauth"),
        };
        let _ = STORE.set(store.clone());
        store
    }

    fn path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn read(&self, id: &str) -> Option<TokenFile> {
        let raw = std::fs::read_to_string(self.path(id)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn write(&self, id: &str, file: &TokenFile) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.path(id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(file)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(FILE_MODE))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn has_token(&self, id: &str) -> bool {
        self.read(id)
            .is_some_and(|file| !file.access_token.trim().is_empty())
    }

    pub fn access_token(&self, id: &str) -> Option<String> {
        let mut file = self.read(id)?;
        if file.access_token.trim().is_empty() {
            return None;
        }
        if token_expired(&file) {
            if let Ok(refreshed) = refresh_token(&file) {
                let _ = self.write(id, &refreshed);
                file = refreshed;
            }
        }
        let token = file.access_token.trim();
        (!token.is_empty()).then(|| token.to_string())
    }

    pub fn disconnect(&self, id: &str) -> anyhow::Result<()> {
        let _ = std::fs::remove_file(self.path(id));
        Ok(())
    }

    pub fn store_access_token(&self, id: &str, token: &str) -> anyhow::Result<()> {
        let token = token.trim();
        if token.is_empty() {
            bail!("token is required");
        }
        self.write(
            id,
            &TokenFile {
                access_token: token.to_string(),
                ..TokenFile::default()
            },
        )
    }

    pub fn annotate(&self, servers: &mut [McpServer]) {
        for server in servers {
            if !matches!(server.transport, McpTransport::Http) {
                server.auth = "none".into();
                continue;
            }
            if self.has_token(&server.id) {
                server.auth = "authorized".into();
                server.auth_account = Some("Connected".into());
                continue;
            }
            if server.url.as_deref().is_some_and(is_public_mcp_url) {
                server.auth = "public".into();
                continue;
            }
            server.auth = "needed".into();
        }
    }

    pub fn start_authorize(&self, server: &McpServer) -> anyhow::Result<Option<String>> {
        let url = server
            .url
            .as_deref()
            .ok_or_else(|| anyhow!("this MCP server has no URL"))?;
        if !matches!(server.transport, McpTransport::Http) {
            bail!("only HTTP MCP servers can sign in");
        }
        let discovered = discover(url)?;
        if discovered.public {
            return Ok(None);
        }
        let Some(registration) = discovered.registration_endpoint.as_deref() else {
            if is_github_mcp(url, &discovered) {
                return self.start_github_authorize(server, &discovered);
            }
            bail!(
                "{} does not advertise a way for Waku to register as an OAuth client",
                server.name
            );
        };
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let registered = register_client(registration, &redirect_uri, &discovered.resource)?;
        let verifier = pkce_verifier();
        let challenge = pkce_challenge(&verifier);
        let state = random_token(16);
        let mut authorize = url::Url::parse(&discovered.authorization_endpoint)?;
        {
            let mut query = authorize.query_pairs_mut();
            query.append_pair("response_type", "code");
            query.append_pair("client_id", &registered.client_id);
            query.append_pair("redirect_uri", &redirect_uri);
            query.append_pair("state", &state);
            query.append_pair("code_challenge", &challenge);
            query.append_pair("code_challenge_method", "S256");
            query.append_pair("resource", &discovered.resource);
            if !discovered.scope.is_empty() {
                query.append_pair("scope", &discovered.scope);
            }
        }
        let pending_auth = PendingAuth {
            token_endpoint: discovered.token_endpoint,
            client_id: registered.client_id,
            client_secret: registered.client_secret,
            redirect_uri,
            code_verifier: verifier,
            resource: discovered.resource,
            state,
        };
        pending()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(server.id.clone(), pending_auth);
        let store = self.clone();
        let id = server.id.clone();
        thread::Builder::new()
            .name("waku-mcp-oauth".into())
            .spawn(move || wait_for_callback(listener, store, id))
            .map_err(|error| anyhow!("could not start sign-in: {error}"))?;
        Ok(Some(authorize.into()))
    }

    fn start_github_authorize(
        &self,
        server: &McpServer,
        discovered: &Discovered,
    ) -> anyhow::Result<Option<String>> {
        if let Some(token) = github_cli_token() {
            self.store_access_token(&server.id, &token)?;
            return Ok(None);
        }
        Ok(Some(github_pat_url(&discovered.scope)))
    }
}

pub fn access_token(id: &str) -> Option<String> {
    STORE.get().and_then(|store| store.access_token(id))
}

struct Discovered {
    public: bool,
    resource: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    scope: String,
}

struct Registered {
    client_id: String,
    client_secret: Option<String>,
}

fn is_public_mcp_url(url: &str) -> bool {
    url.contains("docs.mcp.cloudflare.com")
}

fn is_github_mcp(url: &str, discovered: &Discovered) -> bool {
    url.contains("api.githubcopilot.com")
        || discovered
            .authorization_endpoint
            .contains("github.com/login/oauth")
        || discovered.token_endpoint.contains("github.com/login/oauth")
}

fn github_pat_url(scope: &str) -> String {
    let scopes = if scope.trim().is_empty() {
        "repo,read:org,read:user,user:email,gist,workflow".to_string()
    } else {
        scope.split_whitespace().collect::<Vec<_>>().join(",")
    };
    format!("https://github.com/settings/tokens/new?description=Waku%20MCP&scopes={scopes}")
}

fn github_cli_token() -> Option<String> {
    for bin in ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
        let output = match std::process::Command::new(bin)
            .args(["auth", "token", "-h", "github.com"])
            .output()
        {
            Ok(output) => output,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let Ok(token) = String::from_utf8(output.stdout) else {
            continue;
        };
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn discover(mcp_url: &str) -> anyhow::Result<Discovered> {
    let client = http();
    let initialize = client
        .post(mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"waku","version":"0.1"}}}"#)
        .send();
    let (status, www, _) = match initialize {
        Ok(response) => (
            response.status().as_u16(),
            response
                .headers()
                .get("www-authenticate")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            response,
        ),
        Err(error) => bail!("could not reach MCP server: {error}"),
    };
    if status == 200 {
        return Ok(Discovered {
            public: true,
            resource: mcp_url.to_string(),
            authorization_endpoint: String::new(),
            token_endpoint: String::new(),
            registration_endpoint: None,
            scope: String::new(),
        });
    }
    let parsed = url::Url::parse(mcp_url)?;
    let mut metadata_url = www
        .as_deref()
        .and_then(parse_resource_metadata)
        .unwrap_or_default();
    if metadata_url.is_empty() {
        let origin = format!(
            "{}://{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or_default()
        );
        let path = parsed.path().trim_end_matches('/');
        for candidate in [
            format!("{origin}/.well-known/oauth-protected-resource{path}"),
            format!("{origin}/.well-known/oauth-protected-resource"),
        ] {
            if let Ok(response) = client.get(&candidate).send() {
                if response.status().is_success() {
                    metadata_url = candidate;
                    break;
                }
            }
        }
    }
    if metadata_url.is_empty() {
        bail!("this MCP server did not advertise a sign-in page");
    }
    let prm: serde_json::Value = client.get(&metadata_url).send()?.json()?;
    let resource = prm
        .get("resource")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(mcp_url)
        .to_string();
    let issuer = prm
        .get("authorization_servers")
        .and_then(serde_json::Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("MCP server did not name an authorization server"))?;
    let scope = prm
        .get("scopes_supported")
        .and_then(serde_json::Value::as_array)
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let as_meta = fetch_as_metadata(&client, issuer)?;
    Ok(Discovered {
        public: false,
        resource,
        authorization_endpoint: required_str(&as_meta, "authorization_endpoint")?,
        token_endpoint: required_str(&as_meta, "token_endpoint")?,
        registration_endpoint: as_meta
            .get("registration_endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| guess_registration(issuer)),
        scope,
    })
}

fn fetch_as_metadata(
    client: &reqwest::blocking::Client,
    issuer: &str,
) -> anyhow::Result<serde_json::Value> {
    let parsed = url::Url::parse(issuer)?;
    let origin = format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default()
    );
    let path = parsed.path().trim_end_matches('/');
    let candidates = if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration{path}"),
        ]
    };
    for candidate in candidates {
        if let Ok(response) = client.get(&candidate).send() {
            if response.status().is_success() {
                return Ok(response.json()?);
            }
        }
    }
    bail!("could not load OAuth metadata for {issuer}")
}

fn guess_registration(issuer: &str) -> Option<String> {
    let issuer = issuer.trim_end_matches('/');
    if issuer == "https://vercel.com" {
        return Some("https://vercel.com/api/login/oauth/register".into());
    }
    None
}

fn register_client(
    endpoint: &str,
    redirect_uri: &str,
    resource: &str,
) -> anyhow::Result<Registered> {
    let body = serde_json::json!({
        "client_name": "Waku",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "client_uri": "https://github.com/egoist/waku",
        "resource": resource,
    });
    let response = http()
        .post(endpoint)
        .json(&body)
        .send()
        .map_err(|error| anyhow!("could not register with the sign-in server: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response.json().unwrap_or_else(|_| serde_json::json!({}));
    if !status.is_success() {
        let message = value
            .get("error_description")
            .or_else(|| value.get("error"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("dynamic client registration was rejected");
        bail!("{message}");
    }
    let client_id = value
        .get("client_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("sign-in server returned no client id"))?
        .to_string();
    Ok(Registered {
        client_id,
        client_secret: value
            .get("client_secret")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

fn wait_for_callback(listener: TcpListener, store: OAuthStore, id: String) {
    let _ = listener.set_nonblocking(true);
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        };
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let _ = stream.write_all(CALLBACK_HTML);
        let Some(query) = path.split_once('?').map(|(_, query)| query) else {
            continue;
        };
        let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let Some(pending_auth) = pending()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id)
        else {
            return;
        };
        if params.get("state").map(String::as_str) != Some(pending_auth.state.as_str()) {
            return;
        }
        if let Some(error) = params.get("error") {
            let _ = error;
            return;
        }
        let Some(code) = params.get("code") else {
            return;
        };
        if let Ok(file) = exchange_code(&pending_auth, code) {
            let _ = store.write(&id, &file);
        }
        return;
    }
}

fn exchange_code(pending: &PendingAuth, code: &str) -> anyhow::Result<TokenFile> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("client_id", pending.client_id.clone()),
        ("code_verifier", pending.code_verifier.clone()),
        ("resource", pending.resource.clone()),
    ];
    if let Some(secret) = &pending.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    token_request(&pending.token_endpoint, &form, pending)
}

fn refresh_token(file: &TokenFile) -> anyhow::Result<TokenFile> {
    let refresh = file
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("no refresh token"))?;
    let endpoint = file
        .token_endpoint
        .as_deref()
        .ok_or_else(|| anyhow!("no token endpoint"))?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh.to_string()),
        ("client_id", file.client_id.clone()),
    ];
    if let Some(resource) = &file.resource {
        form.push(("resource", resource.clone()));
    }
    if let Some(secret) = &file.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let pending = PendingAuth {
        token_endpoint: endpoint.to_string(),
        client_id: file.client_id.clone(),
        client_secret: file.client_secret.clone(),
        redirect_uri: String::new(),
        code_verifier: String::new(),
        resource: file.resource.clone().unwrap_or_default(),
        state: String::new(),
    };
    token_request(endpoint, &form, &pending)
}

fn token_request(
    endpoint: &str,
    form: &[(&str, String)],
    pending: &PendingAuth,
) -> anyhow::Result<TokenFile> {
    let response = http()
        .post(endpoint)
        .form(form)
        .send()
        .map_err(|error| anyhow!("token request failed: {error}"))?;
    let status = response.status();
    let value: serde_json::Value = response.json().unwrap_or_else(|_| serde_json::json!({}));
    if !status.is_success() {
        let message = value
            .get("error_description")
            .or_else(|| value.get("error"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("sign-in failed");
        bail!("{message}");
    }
    let access = value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("sign-in server returned no access token"))?;
    let expires_in = value
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(3600);
    Ok(TokenFile {
        client_id: pending.client_id.clone(),
        client_secret: pending.client_secret.clone(),
        access_token: access.to_string(),
        refresh_token: value
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        expires_at: Some(now_secs().saturating_add(expires_in.saturating_sub(30))),
        resource: (!pending.resource.is_empty()).then(|| pending.resource.clone()),
        token_endpoint: Some(pending.token_endpoint.clone()),
    })
}

fn token_expired(file: &TokenFile) -> bool {
    file.expires_at.is_some_and(|expires| expires <= now_secs())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("waku-mcp-oauth")
        .build()
        .expect("reqwest client")
}

fn parse_resource_metadata(header: &str) -> Option<String> {
    let marker = "resource_metadata=";
    let rest = header.split(marker).nth(1)?;
    let value = rest
        .trim_start_matches('"')
        .split(['"', ',', ' '])
        .next()?
        .trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn required_str(value: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("OAuth metadata is missing {key}"))
}

fn pkce_verifier() -> String {
    random_token(32)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn random_token(bytes: usize) -> String {
    let mut raw = Vec::new();
    while raw.len() < bytes {
        raw.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    raw.truncate(bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resource_metadata_header() {
        let header = r#"Bearer realm="OAuth", resource_metadata="https://mcp.cloudflare.com/.well-known/oauth-protected-resource/mcp""#;
        assert_eq!(
            parse_resource_metadata(header).as_deref(),
            Some("https://mcp.cloudflare.com/.well-known/oauth-protected-resource/mcp")
        );
    }

    #[test]
    fn pkce_is_url_safe() {
        let verifier = pkce_verifier();
        let challenge = pkce_challenge(&verifier);
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn public_docs_need_no_login() {
        assert!(is_public_mcp_url("https://docs.mcp.cloudflare.com/mcp"));
        assert!(!is_public_mcp_url("https://mcp.cloudflare.com/mcp"));
    }

    #[test]
    fn github_hosted_mcp_is_detected() {
        let discovered = Discovered {
            public: false,
            resource: "https://api.githubcopilot.com/mcp/".into(),
            authorization_endpoint: "https://github.com/login/oauth/authorize".into(),
            token_endpoint: "https://github.com/login/oauth/access_token".into(),
            registration_endpoint: None,
            scope: "repo read:org".into(),
        };
        assert!(is_github_mcp(
            "https://api.githubcopilot.com/mcp/",
            &discovered
        ));
        assert!(!is_github_mcp(
            "https://mcp.vercel.com",
            &Discovered {
                public: false,
                resource: "https://mcp.vercel.com".into(),
                authorization_endpoint: "https://mcp.vercel.com/authorize".into(),
                token_endpoint: "https://mcp.vercel.com/token".into(),
                registration_endpoint: None,
                scope: String::new(),
            }
        ));
    }

    #[test]
    fn github_pat_url_lists_scopes() {
        let url = github_pat_url("repo read:org");
        assert!(url.starts_with("https://github.com/settings/tokens/new?"));
        assert!(url.contains("scopes=repo,read:org"));
    }
}
