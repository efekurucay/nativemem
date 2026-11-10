use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, Tool};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};

use crate::config::{ServerSpec, Transport};
use crate::error::{HubError, Result};
use crate::secrets::{resolve_value, SecretStore};

/// How long one upstream gets to connect and list its tools.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub type Client = RunningService<RoleClient, ClientConfig>;

/// A live upstream: the session and the tools it lists.
pub struct Connected {
    pub client: Client,
    pub tools: Vec<Tool>,
}

/// A spec with every `secret:<id>` replaced, and the secret values that went in.
struct Resolved {
    spec: ServerSpec,
    secrets: Vec<String>,
}

fn resolve(spec: &ServerSpec, store: &dyn SecretStore) -> Result<Resolved> {
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

async fn open(spec: &ServerSpec) -> std::result::Result<Client, String> {
    match spec.transport {
        Transport::Stdio => {
            let command = spec.command.as_deref().unwrap_or_default();
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(&spec.args).envs(&spec.env);
            if let Some(cwd) = &spec.cwd {
                cmd.current_dir(cwd);
            }
            let transport = TokioChildProcess::new(cmd)
                .map_err(|e| format!("cannot start {command:?}: {e}"))?;
            client_config()
                .serve(transport)
                .await
                .map_err(|e| e.to_string())
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
                        .ok_or("the Authorization header must be a Bearer token")?;
                    config = config.auth_header(token.to_string());
                    continue;
                }
                let name = http::HeaderName::from_bytes(k.as_bytes())
                    .map_err(|_| format!("invalid header name {k:?}"))?;
                let value = http::HeaderValue::from_str(v)
                    .map_err(|_| format!("invalid value for header {k:?}"))?;
                headers.insert(name, value);
            }
            let transport =
                StreamableHttpClientTransport::from_config(config.custom_headers(headers));
            client_config()
                .serve(transport)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// Connect to one upstream and list its tools. The error text never holds a secret value.
pub async fn connect(
    spec: &ServerSpec,
    store: Arc<dyn SecretStore>,
) -> std::result::Result<Connected, String> {
    let spec = spec.clone();
    let resolved = tokio::task::spawn_blocking(move || resolve(&spec, store.as_ref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let work = async {
        let client = open(&resolved.spec).await?;
        match client.peer().list_all_tools().await {
            Ok(tools) => Ok(Connected { client, tools }),
            Err(e) => {
                let _ = client.cancel().await;
                Err(format!("cannot list tools: {e}"))
            }
        }
    };
    match tokio::time::timeout(CONNECT_TIMEOUT, work).await {
        Ok(result) => result.map_err(|m| scrub(m, &resolved.secrets)),
        Err(_) => Err(format!(
            "no answer within {} seconds",
            CONNECT_TIMEOUT.as_secs()
        )),
    }
}

/// What `mcp test` reports for one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    pub ok: bool,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// Connect, list the tools, disconnect.
pub fn test_server(spec: &ServerSpec, store: Arc<dyn SecretStore>) -> Result<TestReport> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| invalid(format!("cannot start the async runtime: {e}")))?;
    Ok(runtime.block_on(async {
        match connect(spec, store).await {
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
                error: Some(error),
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
