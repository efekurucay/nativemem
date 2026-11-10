//! OAuth sign-in for http upstreams: the MCP authorization flow (authorization code with PKCE,
//! protected resource metadata, authorization server metadata, dynamic client registration) on
//! top of `rmcp`'s `auth` module, with the client and the tokens kept in the secret store.
//!
//! Every agent runs its own hub process, so a refresh takes the file lock `<state dir>/hub-oauth.lock`
//! and reads the store again after taking it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationCallback, AuthorizationManager, AuthorizationRequest,
    AuthorizationSession, CredentialRefreshGuard, CredentialStore, OAuthClientConfig,
    StoredCredentials,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::{Auth, ServerSpec, Transport};
use crate::error::{HubError, Result};
use crate::secrets::SecretStore;

/// File name of the lock that serialises token refreshes across hub processes.
pub const LOCK_FILE: &str = "hub-oauth.lock";

/// How long `login` waits for the browser to come back.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a refresh waits for another process that holds the lock.
const LOCK_WAIT: Duration = Duration::from_secs(30);

/// The name the client registers under.
const CLIENT_NAME: &str = "nativemem";

/// How long one browser connection gets to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

pub fn client_secret_id(server: &str) -> String {
    format!("oauth-{server}-client")
}

pub fn tokens_secret_id(server: &str) -> String {
    format!("oauth-{server}-tokens")
}

/// What `mcp list` shows for a server's sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthState {
    SignedIn,
    NeedsLogin,
    None,
}

impl AuthState {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthState::SignedIn => "signed_in",
            AuthState::NeedsLogin => "needs_login",
            AuthState::None => "none",
        }
    }
}

/// `signed_in` means tokens are stored; the authorization server can still have revoked them.
pub fn auth_state(name: &str, spec: &ServerSpec, store: &dyn SecretStore) -> Result<AuthState> {
    if spec.auth != Auth::Oauth {
        return Ok(AuthState::None);
    }
    Ok(if store.get(&tokens_secret_id(name))?.is_some() {
        AuthState::SignedIn
    } else {
        AuthState::NeedsLogin
    })
}

/// The text of an error result for a server that has to be signed in again.
pub fn sign_in_message(name: &str) -> String {
    format!("Sign in again: nativemem mcp login {name}")
}

/// Deletes the stored tokens and keeps the registered client. `false` when nothing was stored.
pub fn logout(name: &str, store: &dyn SecretStore, state: &Path) -> Result<bool> {
    let _lock = lock_exclusive(state).map_err(lock_error)?;
    store.delete(&tokens_secret_id(name))
}

/// The access and refresh tokens stored for a server, so a message can be scrubbed of them.
pub fn token_values(name: &str, store: &dyn SecretStore) -> Vec<String> {
    let Ok(Some(text)) = store.get(&tokens_secret_id(name)) else {
        return Vec::new();
    };
    let Ok(stored) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    ["access_token", "refresh_token"]
        .iter()
        .filter_map(|key| stored["token_response"][key].as_str())
        .map(str::to_string)
        .collect()
}

/// Whether an error text from the transport says the server wants a (new) sign-in.
pub fn is_auth_required(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("auth required") || message.contains("authorization required")
}

fn lock_error(e: std::io::Error) -> HubError {
    HubError::Invalid(format!("cannot take {LOCK_FILE}: {e}"))
}

/// Take the refresh lock, waiting up to `LOCK_WAIT`. The lock is released when the file drops.
fn lock_exclusive(state: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(state)?;
    let file = File::options()
        .create(true)
        .append(true)
        .open(state.join(LOCK_FILE))?;
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "another nativemem process holds the lock",
                ))
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

/// The client a server registered for us. The redirect URI keeps the port, so the next login can
/// bind it again and reuse the registration.
#[derive(Clone, Serialize, Deserialize)]
struct StoredClient {
    client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    redirect_uri: String,
}

fn load_client(store: &dyn SecretStore, name: &str) -> Result<Option<StoredClient>> {
    match store.get(&client_secret_id(name))? {
        None => Ok(None),
        // An unreadable record is treated as no client: login registers a new one.
        Some(text) => Ok(serde_json::from_str(&text).ok()),
    }
}

fn save_client(store: &dyn SecretStore, name: &str, client: &StoredClient) -> Result<()> {
    let text = serde_json::to_string(client)
        .map_err(|e| HubError::SecretStore(format!("cannot encode the client: {e}")))?;
    store.set(&client_secret_id(name), &text)
}

/// The tokens of one server in the secret store, in `rmcp`'s credential format.
struct HubCredentials {
    name: String,
    store: Arc<dyn SecretStore>,
    state: PathBuf,
}

impl HubCredentials {
    fn failed(e: impl std::fmt::Display) -> AuthError {
        AuthError::CredentialStoreError(e.to_string())
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn SecretStore, &str) -> Result<T> + Send + 'static,
    ) -> std::result::Result<T, AuthError> {
        let store = self.store.clone();
        let id = tokens_secret_id(&self.name);
        tokio::task::spawn_blocking(move || work(store.as_ref(), &id))
            .await
            .map_err(Self::failed)?
            .map_err(Self::failed)
    }
}

#[async_trait]
impl CredentialStore for HubCredentials {
    async fn load(&self) -> std::result::Result<Option<StoredCredentials>, AuthError> {
        let text = self.blocking(|store, id| store.get(id)).await?;
        text.map(|t| serde_json::from_str(&t).map_err(Self::failed))
            .transpose()
    }

    async fn save(&self, credentials: StoredCredentials) -> std::result::Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(Self::failed)?;
        self.blocking(move |store, id| store.set(id, &text)).await
    }

    async fn clear(&self) -> std::result::Result<(), AuthError> {
        self.blocking(|store, id| store.delete(id).map(|_| ()))
            .await
    }

    /// The authorization server refused the refresh token for good: the tokens are deleted, unless
    /// another process has stored new ones since. The caller holds the refresh lock.
    async fn on_refresh_token_rejected(
        &self,
        credentials: &StoredCredentials,
    ) -> std::result::Result<(), AuthError> {
        let encode = |c: &StoredCredentials| serde_json::to_string(c).map_err(Self::failed);
        match self.load().await? {
            Some(current) if encode(&current)? == encode(credentials)? => self.clear().await,
            _ => Ok(()),
        }
    }

    async fn acquire_refresh_guard(
        &self,
    ) -> std::result::Result<Option<CredentialRefreshGuard>, AuthError> {
        let state = self.state.clone();
        let file = tokio::task::spawn_blocking(move || lock_exclusive(&state))
            .await
            .map_err(Self::failed)?
            .map_err(Self::failed)?;
        Ok(Some(CredentialRefreshGuard::new(file)))
    }
}

fn manager_error(e: AuthError) -> String {
    e.to_string()
}

/// A request client that attaches the stored bearer token and refreshes it under the lock.
/// `Err(None)` means the server is not signed in; `Err(Some(message))` is any other failure.
pub async fn authorized_client(
    name: &str,
    url: &str,
    store: Arc<dyn SecretStore>,
    state: &Path,
) -> std::result::Result<AuthClient<reqwest::Client>, Option<String>> {
    let credentials = HubCredentials {
        name: name.to_string(),
        store: store.clone(),
        state: state.to_path_buf(),
    };
    if credentials
        .load()
        .await
        .map_err(|e| Some(manager_error(e)))?
        .is_none()
    {
        return Err(None);
    }
    let client = {
        let store = store.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || load_client(store.as_ref(), &name))
            .await
            .map_err(|e| Some(e.to_string()))?
            .map_err(|e| Some(e.to_string()))?
    };
    let Some(client) = client else {
        return Err(None);
    };
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|e| Some(manager_error(e)))?;
    manager.set_credential_store(credentials);
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|e| Some(manager_error(e)))?;
    manager.set_metadata(resolution.metadata);
    let mut config = OAuthClientConfig::new(client.client_id, client.redirect_uri);
    if let Some(secret) = client.client_secret {
        config = config.with_client_secret(secret);
    }
    manager
        .configure_client(config)
        .map_err(|e| Some(manager_error(e)))?;
    // The token is read, and refreshed when it is about to expire, before the first request.
    match manager.get_access_token().await {
        Ok(_) => {}
        Err(AuthError::AuthorizationRequired) => return Err(None),
        Err(e) => return Err(Some(manager_error(e))),
    }
    Ok(AuthClient::new(reqwest::Client::new(), manager))
}

/// What `login` needs besides the server: how to show the authorization URL and how long to wait.
pub struct LoginOptions<'a> {
    pub timeout: Duration,
    /// Called once with the authorization URL after the loopback listener is up.
    pub open: &'a (dyn Fn(&str) + Send + Sync),
}

fn invalid(message: impl Into<String>) -> HubError {
    HubError::Invalid(message.into())
}

fn auth_failure(context: &str, e: AuthError) -> HubError {
    invalid(format!("{context}: {e}"))
}

/// Sign in to one OAuth server and store the client and the tokens.
pub fn login(
    name: &str,
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
    state: &Path,
    options: &LoginOptions,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| invalid(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(login_async(name, spec, store, state, options))
}

async fn login_async(
    name: &str,
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
    state: &Path,
    options: &LoginOptions<'_>,
) -> Result<()> {
    if spec.transport != Transport::Http || spec.auth != Auth::Oauth {
        return Err(invalid(format!(
            "server {name:?} does not use auth = \"oauth\""
        )));
    }
    let resolved = {
        let (spec, store) = (spec.clone(), store.clone());
        tokio::task::spawn_blocking(move || crate::connect::resolve(&spec, store.as_ref()))
            .await
            .map_err(|e| invalid(e.to_string()))??
    };
    let url = resolved.spec.url.clone().unwrap_or_default();
    let challenge = probe(&url, &resolved.spec).await?;

    let mut manager = AuthorizationManager::new(url.as_str())
        .await
        .map_err(|e| auth_failure("cannot reach the server", e))?;
    manager.set_credential_store(HubCredentials {
        name: name.to_string(),
        store: store.clone(),
        state: state.to_path_buf(),
    });
    let resolution = manager
        .resolve_metadata_from_challenge(challenge.as_deref())
        .await
        .map_err(|e| auth_failure("cannot discover the authorization server", e))?;
    manager.set_metadata(resolution.metadata);

    let known = load_client_blocking(&store, name).await?;
    let (listener, reused) = bind_loopback(known.as_ref()).await?;
    let port = listener
        .local_addr()
        .map_err(|e| invalid(format!("cannot read the loopback address: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let client = match known.filter(|_| reused) {
        Some(client) => client,
        None => {
            let registered = manager
                .register_client(CLIENT_NAME, &redirect_uri, &[])
                .await
                .map_err(|e| auth_failure("cannot register the client", e))?;
            let client = StoredClient {
                client_id: registered.client_id,
                client_secret: registered.client_secret,
                redirect_uri: redirect_uri.clone(),
            };
            save_client_blocking(&store, name, &client).await?;
            client
        }
    };

    let mut request = AuthorizationRequest::new(&redirect_uri)
        .with_client_name(CLIENT_NAME)
        .with_preregistered_client(&client.client_id)
        .with_initial_scope_selection();
    if let Some(secret) = &client.client_secret {
        request = request.with_client_secret(secret);
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| auth_failure("cannot start the authorization", e))?;

    (options.open)(session.get_authorization_url());
    let callback_url = tokio::time::timeout(options.timeout, wait_for_callback(listener, port))
        .await
        .map_err(|_| {
            invalid(format!(
                "no sign-in within {} seconds",
                options.timeout.as_secs()
            ))
        })??;

    let lock_state = state.to_path_buf();
    let _lock = tokio::task::spawn_blocking(move || lock_exclusive(&lock_state))
        .await
        .map_err(|e| invalid(e.to_string()))?
        .map_err(lock_error)?;
    let scrub = resolved.secrets.clone();
    session
        .handle_callback_url(&callback_url)
        .await
        .map_err(|e| {
            let message = e.to_string();
            invalid(format!(
                "cannot finish the sign-in: {}",
                scrub
                    .iter()
                    .filter(|s| !s.is_empty())
                    .fold(message, |m, s| m.replace(s.as_str(), "***"))
            ))
        })?;
    Ok(())
}

async fn load_client_blocking(
    store: &Arc<dyn SecretStore>,
    name: &str,
) -> Result<Option<StoredClient>> {
    let (store, name) = (store.clone(), name.to_string());
    tokio::task::spawn_blocking(move || load_client(store.as_ref(), &name))
        .await
        .map_err(|e| invalid(e.to_string()))?
}

async fn save_client_blocking(
    store: &Arc<dyn SecretStore>,
    name: &str,
    client: &StoredClient,
) -> Result<()> {
    let (store, name, client) = (store.clone(), name.to_string(), client.clone());
    tokio::task::spawn_blocking(move || save_client(store.as_ref(), &name, &client))
        .await
        .map_err(|e| invalid(e.to_string()))?
}

/// Bind the loopback port of the stored client when it is free, so its registration stays valid;
/// otherwise a random port. The flag says whether the stored client's port was bound.
async fn bind_loopback(known: Option<&StoredClient>) -> Result<(TcpListener, bool)> {
    let remembered = known
        .and_then(|c| c.redirect_uri.strip_prefix("http://127.0.0.1:"))
        .and_then(|rest| rest.split('/').next())
        .and_then(|port| port.parse::<u16>().ok());
    if let Some(port) = remembered {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok((listener, true));
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| invalid(format!("cannot listen on the loopback interface: {e}")))?;
    Ok((listener, false))
}

/// Ask the server for something without a token. A 401 returns its `WWW-Authenticate` header,
/// which names the protected resource metadata; a success means the server needs no sign-in.
async fn probe(url: &str, spec: &ServerSpec) -> Result<Option<String>> {
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "nativemem-hub", "version": env!("CARGO_PKG_VERSION") },
        },
    });
    let mut request = reqwest::Client::new()
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&initialize);
    for (key, value) in &spec.headers {
        request = request.header(key, value);
    }
    let response = tokio::time::timeout(Duration::from_secs(20), request.send())
        .await
        .map_err(|_| invalid("the server did not answer within 20 seconds"))?
        .map_err(|e| invalid(format!("cannot reach the server: {e}")))?;
    if response.status().is_success() {
        return Err(invalid("the server answers without a sign-in"));
    }
    Ok(response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string))
}

const PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>nativemem</title>\
<body style=\"font-family: system-ui; margin: 3rem\">";

async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
    let page = format!("{PAGE}<p>{body}</p></body>");
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
}

/// The request target of the first line of a request, read from one connection.
async fn request_target(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let read = async {
        while !buf.windows(2).any(|w| w == b"\r\n") && buf.len() < 16 * 1024 {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    };
    tokio::time::timeout(REQUEST_TIMEOUT, read).await.ok()?;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

/// Wait for the browser to be sent to `/callback` and return the full redirect URL.
async fn wait_for_callback(listener: TcpListener, port: u16) -> Result<String> {
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let Some(target) = request_target(&mut socket).await else {
            continue;
        };
        if !(target == "/callback" || target.starts_with("/callback?")) {
            respond(&mut socket, "404 Not Found", "Not found.").await;
            continue;
        }
        let url = format!("http://127.0.0.1:{port}{target}");
        match AuthorizationCallback::from_redirect_url(&url) {
            Ok(_) => {
                respond(
                    &mut socket,
                    "200 OK",
                    "Signed in. You can close this tab and return to the terminal.",
                )
                .await;
                return Ok(url);
            }
            Err(_) => {
                let error = url_error_code(&url);
                respond(&mut socket, "400 Bad Request", "Sign-in failed.").await;
                return Err(invalid(match error {
                    Some(code) => {
                        format!("the authorization server answered with an error: {code}")
                    }
                    None => "the sign-in callback carried no authorization code".to_string(),
                }));
            }
        }
    }
}

/// The `error` parameter of a callback URL, when it is a short code and not free text.
fn url_error_code(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let code = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("error="))?;
    code.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then(|| code.to_string())
}

/// Open the URL in the default browser. Failures are ignored: the caller prints the URL too.
pub fn open_browser(url: &str, program: Option<&Path>) {
    let program: PathBuf = match program {
        Some(p) => p.to_path_buf(),
        None if cfg!(target_os = "macos") => PathBuf::from("open"),
        None => PathBuf::from("xdg-open"),
    };
    if let Ok(mut child) = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::FileStore;

    #[test]
    fn secret_ids_derive_from_the_server_name() {
        assert_eq!(client_secret_id("gh"), "oauth-gh-client");
        assert_eq!(tokens_secret_id("gh"), "oauth-gh-tokens");
        let long = "x".repeat(32);
        assert!(crate::secrets::validate_secret_id(&tokens_secret_id(&long)).is_ok());
    }

    #[test]
    fn auth_state_follows_the_stored_tokens() {
        let t = tempfile::tempdir().unwrap();
        let store = FileStore::new(t.path());
        let mut spec = ServerSpec::http("https://mcp.example.com/mcp");
        assert_eq!(
            auth_state("a", &spec, &store).unwrap(),
            AuthState::None,
            "no oauth"
        );
        spec.auth = Auth::Oauth;
        assert_eq!(
            auth_state("a", &spec, &store).unwrap(),
            AuthState::NeedsLogin
        );
        store.set(&tokens_secret_id("a"), "{}").unwrap();
        assert_eq!(auth_state("a", &spec, &store).unwrap(), AuthState::SignedIn);
    }

    #[test]
    fn the_lock_is_exclusive_until_the_file_drops() {
        let t = tempfile::tempdir().unwrap();
        let first = lock_exclusive(t.path()).unwrap();
        let second = File::options()
            .append(true)
            .open(t.path().join(LOCK_FILE))
            .unwrap();
        assert!(second.try_lock().is_err());
        drop(first);
        assert!(second.try_lock().is_ok());
    }

    #[test]
    fn only_short_error_codes_are_reported() {
        assert_eq!(
            url_error_code("http://h/callback?error=access_denied&state=x").as_deref(),
            Some("access_denied")
        );
        assert_eq!(url_error_code("http://h/callback?error=a%20b"), None);
        assert_eq!(url_error_code("http://h/callback"), None);
    }

    #[test]
    fn auth_required_is_recognised_in_transport_errors() {
        assert!(is_auth_required("Send message error: Auth required"));
        assert!(is_auth_required("OAuth authorization required"));
        assert!(!is_auth_required("connection refused"));
    }
}
