use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, Tool};
use rmcp::service::RunningService;
use rmcp::transport::auth::AuthClient;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};

use crate::config::{Auth, ServerSpec, Transport};
use crate::error::{HubError, Result};
use crate::oauth;
use crate::secrets::{resolve_value, SecretStore};

/// How long one upstream gets to connect and list its tools.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub type Client = RunningService<RoleClient, ClientConfig>;

/// A live upstream: the session and the tools it lists. `auth` is the token source of an OAuth
/// server.
pub struct Connected {
    pub client: Client,
    pub tools: Vec<Tool>,
    pub auth: Option<AuthClient<reqwest::Client>>,
}

/// Why an upstream is not connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// The OAuth server has no usable sign-in: `nativemem mcp login <name>` fixes it.
    NeedsLogin(String),
    Failed(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::NeedsLogin(name) => f.write_str(&oauth::sign_in_message(name)),
            ConnectError::Failed(message) => f.write_str(message),
        }
    }
}

/// A spec with every `secret:<id>` replaced, and the secret values that went in.
pub(crate) struct Resolved {
    pub(crate) spec: ServerSpec,
    pub(crate) secrets: Vec<String>,
}

pub(crate) fn resolve(spec: &ServerSpec, store: &dyn SecretStore) -> Result<Resolved> {
    let mut secrets = Vec::new();
    let mut one = |v: &str| -> Result<String> {
        let (value, secret) = resolve_value(store, v)?;
        secrets.extend(secret);
        Ok(value)
    };
    let mut out = spec.clone();
    out.command = spec.command.as_deref().map(&mut one).transpose()?;
    out.args = spec
        .args
        .iter()
        .map(|a| one(a))
        .collect::<Result<Vec<_>>>()?;
    for (k, v) in &spec.env {
        out.env.insert(k.clone(), one(v)?);
    }
    out.cwd = spec.cwd.as_deref().map(&mut one).transpose()?;
    out.url = spec.url.as_deref().map(&mut one).transpose()?;
    for (k, v) in &spec.headers {
        out.headers.insert(k.clone(), one(v)?);
    }
    Ok(Resolved { spec: out, secrets })
}

/// Secret values never reach a message: each is replaced by `***`.
fn scrub(message: String, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|s| !s.is_empty())
        .fold(message, |m, s| m.replace(s.as_str(), "***"))
}

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("nativemem-hub", env!("CARGO_PKG_VERSION")),
    )
}

fn invalid(message: impl Into<String>) -> HubError {
    HubError::Invalid(message.into())
}

/// An open session and, for an OAuth server, the token source behind it.
type Opened = (Client, Option<AuthClient<reqwest::Client>>);

async fn open(
    name: &str,
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
    state: &Path,
) -> std::result::Result<Opened, ConnectError> {
    let failed = |m: String| ConnectError::Failed(m);
    match spec.transport {
        Transport::Stdio => {
            let command = spec.command.as_deref().unwrap_or_default();
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(&spec.args).envs(&spec.env);
            if let Some(cwd) = &spec.cwd {
                cmd.current_dir(cwd);
            }
            let transport = TokioChildProcess::new(cmd)
                .map_err(|e| failed(format!("cannot start {command:?}: {e}")))?;
            let client = client_config()
                .serve(transport)
                .await
                .map_err(|e| failed(e.to_string()))?;
            Ok((client, None))
        }
        Transport::Http => {
            let mut config =
                StreamableHttpClientTransportConfig::with_uri(spec.url.clone().unwrap_or_default());
            let mut headers = HashMap::new();
            for (k, v) in &spec.headers {
                if k.eq_ignore_ascii_case("authorization") {
                    // The transport sends the Authorization header itself and wants the token
                    // without its scheme.
                    let token = v
                        .strip_prefix("Bearer ")
                        .or_else(|| v.strip_prefix("bearer "))
                        .ok_or_else(|| {
                            failed("the Authorization header must be a Bearer token".into())
                        })?;
                    config = config.auth_header(token.to_string());
                    continue;
                }
                let header = http::HeaderName::from_bytes(k.as_bytes())
                    .map_err(|_| failed(format!("invalid header name {k:?}")))?;
                let value = http::HeaderValue::from_str(v)
                    .map_err(|_| failed(format!("invalid value for header {k:?}")))?;
                headers.insert(header, value);
            }
            let config = config.custom_headers(headers);
            if spec.auth == Auth::Oauth {
                let url = spec.url.clone().unwrap_or_default();
                let auth = oauth::authorized_client(name, &url, store, state)
                    .await
                    .map_err(|e| match e {
                        None => ConnectError::NeedsLogin(name.to_string()),
                        Some(message) => failed(message),
                    })?;
                let transport = StreamableHttpClientTransport::with_client(auth.clone(), config);
                let client = client_config()
                    .serve(transport)
                    .await
                    .map_err(|e| failed(e.to_string()))?;
                return Ok((client, Some(auth)));
            }
            let transport = StreamableHttpClientTransport::from_config(config);
            let client = client_config()
                .serve(transport)
                .await
                .map_err(|e| failed(e.to_string()))?;
            Ok((client, None))
        }
    }
}

/// Connect to one upstream and list its tools. The error text never holds a secret value or a token.
pub async fn connect(
    name: &str,
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
    state: &Path,
) -> std::result::Result<Connected, ConnectError> {
    let failed = |m: String| ConnectError::Failed(m);
    let resolved = {
        let (spec, store) = (spec.clone(), store.clone());
        tokio::task::spawn_blocking(move || resolve(&spec, store.as_ref()))
            .await
            .map_err(|e| failed(e.to_string()))?
            .map_err(|e| failed(e.to_string()))?
    };
    let mut secrets = resolved.secrets.clone();
    if spec.auth == Auth::Oauth {
        let (name, store) = (name.to_string(), store.clone());
        secrets.extend(
            tokio::task::spawn_blocking(move || oauth::token_values(&name, store.as_ref()))
                .await
                .unwrap_or_default(),
        );
    }
    let work = async {
        let (client, auth) = open(name, &resolved.spec, store, state).await?;
        match client.peer().list_all_tools().await {
            Ok(tools) => Ok(Connected {
                client,
                tools,
                auth,
            }),
            Err(e) => {
                let _ = client.cancel().await;
                Err(failed(format!("cannot list tools: {e}")))
            }
        }
    };
    let result = match tokio::time::timeout(CONNECT_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_) => Err(failed(format!(
            "no answer within {} seconds",
            CONNECT_TIMEOUT.as_secs()
        ))),
    };
    result.map_err(|e| match e {
        ConnectError::Failed(m) if spec.auth == Auth::Oauth && oauth::is_auth_required(&m) => {
            ConnectError::NeedsLogin(name.to_string())
        }
        ConnectError::Failed(m) => ConnectError::Failed(scrub(m, &secrets)),
        needs_login => needs_login,
    })
}

/// What `mcp test` reports for one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    pub ok: bool,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// Connect, list the tools, disconnect.
pub fn test_server(
    name: &str,
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
    state: &Path,
) -> Result<TestReport> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| invalid(format!("cannot start the async runtime: {e}")))?;
    Ok(runtime.block_on(async {
        match connect(name, spec, store, state).await {
            Ok(c) => {
                let tools = c.tools.iter().map(|t| t.name.to_string()).collect();
                let _ = c.client.cancel().await;
                TestReport {
                    ok: true,
                    tools,
                    error: None,
                }
            }
            Err(error) => TestReport {
                ok: false,
                tools: Vec::new(),
                error: Some(error.to_string()),
            },
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::FileStore;

    #[test]
    fn resolve_replaces_every_reference() {
        let t = tempfile::tempdir().unwrap();
        let store = FileStore::new(t.path());
        store.set("a", "AAA").unwrap();
        store.set("b", "BBB").unwrap();
        let mut spec = ServerSpec::stdio("tool");
        spec.args = vec!["--k=1".into(), "secret:a".into()];
        spec.env.insert("T".into(), "secret:b".into());
        let r = resolve(&spec, &store).unwrap();
        assert_eq!(r.spec.args, vec!["--k=1", "AAA"]);
        assert_eq!(r.spec.env["T"], "BBB");
        assert_eq!(r.secrets, vec!["AAA", "BBB"]);
        spec.env.insert("U".into(), "secret:none".into());
        assert!(resolve(&spec, &store).is_err());
    }

    #[test]
    fn scrub_hides_secret_values() {
        let m = scrub(
            "bad token AAA for BBB".into(),
            &["AAA".into(), "BBB".into()],
        );
        assert_eq!(m, "bad token *** for ***");
    }
}
