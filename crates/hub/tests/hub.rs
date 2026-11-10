//! The hub against a fake stdio MCP server (`nativemem-fake-mcp`), over in-memory transports.
//! Secrets live in a file store inside a temp directory.

use std::path::Path;
use std::sync::Arc;

use nativemem_hub::config::{HubConfig, ServerSpec};
use nativemem_hub::secrets::{FileStore, SecretStore};
use nativemem_hub::serve::HubServer;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

const FAKE: &str = env!("CARGO_BIN_EXE_nativemem-fake-mcp");

fn fake() -> ServerSpec {
    ServerSpec::stdio(FAKE)
}

/// A client connected to an in-process hub.
async fn hub(
    config: HubConfig,
    store: Arc<dyn SecretStore>,
    state: &Path,
) -> RunningService<RoleClient, ClientConfig> {
    let state = state.to_path_buf();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(server) = HubServer::new(config, store, state).serve(server_io).await {
            let _ = server.waiting().await;
        }
    });
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(client_io)
    .await
    .expect("the hub initialises")
}

fn args(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap()
}

async fn call_text(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &str,
    arguments: Value,
) -> String {
    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args(arguments)))
        .await
        .expect("the call succeeds");
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn store(dir: &Path) -> Arc<dyn SecretStore> {
    Arc::new(FileStore::new(dir))
}

#[tokio::test]
async fn tools_are_prefixed_and_calls_are_forwarded_unchanged() {
    let t = tempfile::tempdir().unwrap();
    let mut config = HubConfig::default();
    config.servers.insert("fake".into(), fake());
    let client = hub(config, store(t.path()), t.path()).await;

    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        [
            "nativemem_status",
            "subagents_list",
            "fake__echo",
            "fake__env_matches"
        ]
    );
    let echo = tools.iter().find(|t| t.name == "fake__echo").unwrap();
    assert_eq!(echo.description.as_deref(), Some("[fake] Returns its text"));

    assert_eq!(
        call_text(&client, "fake__echo", json!({ "text": "hello" })).await,
        "hello"
    );
    assert!(client
        .peer()
        .call_tool(CallToolRequestParams::new("fake__nope".to_string()))
        .await
        .is_err());
}

#[tokio::test]
async fn failing_and_disabled_upstreams_are_reported_and_left_out() {
    let t = tempfile::tempdir().unwrap();
    let mut config = HubConfig::default();
    config.servers.insert("good".into(), fake());
    config.servers.insert(
        "broken".into(),
        ServerSpec::stdio("/nonexistent/nativemem-test-binary"),
    );
    config
        .servers
        .insert("refused".into(), ServerSpec::http("http://127.0.0.1:1/mcp"));
    let mut off = fake();
    off.enabled = false;
    config.servers.insert("off".into(), off);
    let client = hub(config, store(t.path()), t.path()).await;

    let tools = client.peer().list_all_tools().await.unwrap();
    assert!(tools.iter().all(|t| {
        let n = t.name.as_ref();
        n == "nativemem_status" || n == "subagents_list" || n.starts_with("good__")
    }));

    let status: Value =
        serde_json::from_str(&call_text(&client, "nativemem_status", json!({})).await).unwrap();
    let by_name = |name: &str| {
        status["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no status for {name}: {status}"))
    };
    assert_eq!(by_name("good")["status"], "connected");
    assert_eq!(by_name("good")["tools"], 2);
    assert_eq!(by_name("off")["status"], "disabled");
    for failing in ["broken", "refused"] {
        let s = by_name(failing);
        assert_eq!(s["status"], "error", "{s}");
        assert!(!s["error"].as_str().unwrap().is_empty(), "{s}");
    }
}

#[tokio::test]
async fn secret_references_reach_the_child_and_never_the_output() {
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    secrets.set("tok", "s3cr3t-value").unwrap();
    let mut spec = fake();
    spec.env.insert("API_TOKEN".into(), "secret:tok".into());
    spec.env.insert("MISSING".into(), "secret:absent".into());
    let mut with_missing = HubConfig::default();
    with_missing.servers.insert("bad".into(), spec.clone());
    spec.env.remove("MISSING");
    let mut config = HubConfig::default();
    config.servers.insert("fake".into(), spec);
    config
        .servers
        .insert("bad".into(), with_missing.servers["bad"].clone());
    let client = hub(config, secrets, t.path()).await;

    assert_eq!(
        call_text(
            &client,
            "fake__env_matches",
            json!({ "name": "API_TOKEN", "value": "s3cr3t-value" })
        )
        .await,
        "true"
    );
    assert_eq!(
        call_text(
            &client,
            "fake__env_matches",
            json!({ "name": "API_TOKEN", "value": "other" })
        )
        .await,
        "false"
    );

    let listing = serde_json::to_string(&client.peer().list_all_tools().await.unwrap()).unwrap();
    let status = call_text(&client, "nativemem_status", json!({})).await;
    assert!(!listing.contains("s3cr3t-value"), "{listing}");
    assert!(!status.contains("s3cr3t-value"), "{status}");
    // A reference whose secret is missing names the id, not a value.
    assert!(status.contains("absent"), "{status}");
}

#[tokio::test]
async fn http_upstream_receives_resolved_headers() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(std::sync::Mutex::new(String::new()));
    let seen_by_server = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen_by_server.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16 * 1024];
                let mut request = String::new();
                // Read until the JSON body is complete.
                while let Ok(n) = socket.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if let Some((head, body)) = request.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if body.len() >= len {
                            break;
                        }
                    }
                }
                seen.lock().unwrap().push_str(&request);
                let body = if request.contains("\"initialize\"") {
                    let id =
                        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1)
                            .map(|v| v["id"].clone())
                            .unwrap_or(Value::Null);
                    json!({"jsonrpc":"2.0","id":id,"result":{
                        "protocolVersion":"2025-06-18","capabilities":{"tools":{}},
                        "serverInfo":{"name":"raw","version":"0"}}})
                    .to_string()
                } else if request.contains("tools/list") {
                    let id =
                        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1)
                            .map(|v| v["id"].clone())
                            .unwrap_or(Value::Null);
                    json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
                        {"name":"ping","description":"p","inputSchema":{"type":"object"}}]}})
                    .to_string()
                } else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n")
                        .await;
                    return;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });

    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    secrets.set("api", "Bearer http-secret").unwrap();
    secrets.set("custom", "custom-secret").unwrap();
    let mut spec = ServerSpec::http(format!("http://127.0.0.1:{port}/mcp"));
    spec.headers
        .insert("Authorization".into(), "secret:api".into());
    spec.headers
        .insert("X-Api-Key".into(), "secret:custom".into());
    let connected = nativemem_hub::connect::connect("raw", &spec, secrets, t.path())
        .await
        .unwrap_or_else(|e| panic!("connect failed: {e}"));
    let request = seen.lock().unwrap().to_ascii_lowercase();
    let tools: Vec<&str> = connected.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(tools, ["ping"]);
    assert!(
        request.contains("authorization: bearer http-secret"),
        "{request}"
    );
    assert!(request.contains("x-api-key: custom-secret"), "{request}");
}
