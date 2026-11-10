use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::auth::{AuthClient, AuthError};
use rmcp::{ErrorData as McpError, Peer, RoleClient, RoleServer, ServerHandler, ServiceError};
use serde_json::json;
use tokio::sync::OnceCell;
use tokio::task::JoinSet;

use crate::config::HubConfig;
use crate::connect::{connect, Client, ConnectError, Connected};
use crate::error::{HubError, Result};
use crate::oauth;
use crate::secrets::SecretStore;

/// The tool that reports the state of every configured server.
pub const STATUS_TOOL: &str = "nativemem_status";

/// The tool name a server that is not signed in exposes.
const SIGN_IN_TOOL: &str = "sign_in_required";

const MAX_TOOL_NAME: usize = 64;

/// `<server>__<tool>` limited to the characters and the length MCP clients accept.
pub fn exposed_name(server: &str, tool: &str) -> String {
    let mut name: String = format!("{server}__{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.truncate(MAX_TOOL_NAME);
    name
}

/// `exposed_name`, made unique among `taken` by a numeric suffix.
fn unique_name(server: &str, tool: &str, taken: &BTreeSet<String>) -> String {
    let base = exposed_name(server, tool);
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| {
            let suffix = format!("_{n}");
            let mut head = base.clone();
            head.truncate(MAX_TOOL_NAME - suffix.len());
            head + &suffix
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded range has a free name")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerState {
    Connected {
        tools: usize,
    },
    Error(String),
    /// An OAuth server whose sign-in is missing or no longer works.
    NeedsLogin(String),
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: String,
    pub state: ServerState,
}

impl ServerStatus {
    fn json(&self) -> serde_json::Value {
        match &self.state {
            ServerState::Connected { tools } => {
                json!({ "name": self.name, "status": "connected", "tools": tools })
            }
            ServerState::Error(message) => {
                json!({ "name": self.name, "status": "error", "error": message })
            }
            ServerState::NeedsLogin(message) => {
                json!({ "name": self.name, "status": "needs_login", "error": message })
            }
            ServerState::Disabled => json!({ "name": self.name, "status": "disabled" }),
        }
    }
}

/// Where a call goes.
enum Target {
    Upstream {
        peer: Peer<RoleClient>,
        tool: String,
        /// The token source of an OAuth server.
        auth: Option<AuthClient<reqwest::Client>>,
    },
    /// The one tool of an OAuth server that is not signed in: it says how to sign in.
    SignIn,
}

struct Route {
    server: String,
    target: Target,
}

/// The connected upstreams, the tools they expose and where a call to each goes.
struct Live {
    tools: Vec<Tool>,
    routes: HashMap<String, Route>,
    status: Vec<ServerStatus>,
    /// Dropping a client ends its session.
    _clients: Vec<Client>,
}

async fn connect_all(config: HubConfig, store: Arc<dyn SecretStore>, state: PathBuf) -> Live {
    let mut status: Vec<ServerStatus> = Vec::new();
    let mut signed_out: Vec<String> = Vec::new();
    let mut set = JoinSet::new();
    for (name, spec) in config.servers {
        if !spec.enabled {
            status.push(ServerStatus {
                name,
                state: ServerState::Disabled,
            });
            continue;
        }
        let (store, state) = (store.clone(), state.clone());
        set.spawn(async move {
            let result = connect(&name, &spec, store, &state).await;
            (name, result)
        });
    }
    let mut connected: Vec<(String, Connected)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((name, Ok(c))) => connected.push((name, c)),
            Ok((name, Err(error))) => {
                eprintln!("nativemem hub: {name}: {error}");
                let state = match error {
                    ConnectError::NeedsLogin(_) => {
                        signed_out.push(name.clone());
                        ServerState::NeedsLogin(error.to_string())
                    }
                    ConnectError::Failed(message) => ServerState::Error(message),
                };
                status.push(ServerStatus { name, state });
            }
            Err(e) => eprintln!("nativemem hub: a connection task failed: {e}"),
        }
    }
    // Completion order is arbitrary; the tool list is not.
    connected.sort_by(|a, b| a.0.cmp(&b.0));

    let mut taken: BTreeSet<String> = BTreeSet::from([STATUS_TOOL.to_string()]);
    let mut tools = Vec::new();
    let mut routes = HashMap::new();
    let mut clients = Vec::new();
    for (name, c) in connected {
        status.push(ServerStatus {
            name: name.clone(),
            state: ServerState::Connected {
                tools: c.tools.len(),
            },
        });
        let peer = c.client.peer().clone();
        let auth = c.auth;
        for mut tool in c.tools {
            let upstream = tool.name.to_string();
            let exposed = unique_name(&name, &upstream, &taken);
            taken.insert(exposed.clone());
            tool.name = exposed.clone().into();
            tool.description = Some(
                format!(
                    "[{name}] {}",
                    tool.description.as_deref().unwrap_or_default()
                )
                .into(),
            );
            routes.insert(
                exposed,
                Route {
                    server: name.clone(),
                    target: Target::Upstream {
                        peer: peer.clone(),
                        tool: upstream,
                        auth: auth.clone(),
                    },
                },
            );
            tools.push(tool);
        }
        clients.push(c.client);
    }
    signed_out.sort();
    for name in signed_out {
        let exposed = unique_name(&name, SIGN_IN_TOOL, &taken);
        taken.insert(exposed.clone());
        let tool = Tool::new(
            exposed.clone(),
            format!("[{name}] {}", oauth::sign_in_message(&name)),
            json!({ "type": "object", "properties": {} })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        routes.insert(
            exposed,
            Route {
                server: name,
                target: Target::SignIn,
            },
        );
        tools.push(tool);
    }
    status.sort_by(|a, b| a.name.cmp(&b.name));
    Live {
        tools,
        routes,
        status,
        _clients: clients,
    }
}

fn status_tool() -> Tool {
    let schema = json!({ "type": "object", "properties": {} });
    Tool::new(
        STATUS_TOOL,
        "State of every server behind the nativemem hub: connected, error, needs_login or disabled",
        schema.as_object().cloned().unwrap_or_default(),
    )
}

/// The MCP server the agents talk to. It connects to the upstreams on the first request that
/// needs them, which is `initialize`.
pub struct HubServer {
    config: HubConfig,
    store: Arc<dyn SecretStore>,
    state: PathBuf,
    live: OnceCell<Live>,
}

impl HubServer {
    /// `state` is the state directory: the token refresh lock lives there.
    pub fn new(config: HubConfig, store: Arc<dyn SecretStore>, state: PathBuf) -> Self {
        HubServer {
            config,
            store,
            state,
            live: OnceCell::new(),
        }
    }

    async fn live(&self) -> &Live {
        self.live
            .get_or_init(|| {
                connect_all(self.config.clone(), self.store.clone(), self.state.clone())
            })
            .await
    }
}

fn text_result(text: String, is_error: bool) -> CallToolResponse {
    let content = vec![ContentBlock::text(text)];
    CallToolResponse::Complete(if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    })
}

impl ServerHandler for HubServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            rmcp::model::Implementation::new("nativemem", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request.clone());
        self.live().await;
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        let live = self.live().await;
        let mut tools = vec![status_tool()];
        tools.extend(live.tools.iter().cloned());
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        let live = self.live().await;
        if request.name == STATUS_TOOL {
            let doc = json!({ "servers": live.status.iter().map(ServerStatus::json).collect::<Vec<_>>() });
            return Ok(text_result(doc.to_string(), false));
        }
        let Some(route) = live.routes.get(request.name.as_ref()) else {
            return Err(McpError::invalid_params(
                format!("unknown tool {:?}", request.name),
                None,
            ));
        };
        let Target::Upstream { peer, tool, auth } = &route.target else {
            return Ok(text_result(oauth::sign_in_message(&route.server), true));
        };
        let mut forwarded = request;
        forwarded.name = tool.clone().into();
        match peer.call_tool_once(forwarded).await {
            Ok(response) => Ok(response),
            Err(ServiceError::McpError(e)) => Err(e),
            Err(e) => {
                let message = e.to_string();
                // A token that stopped working mid-session: the refresh failed for good.
                let signed_out = match auth {
                    Some(auth) => {
                        matches!(
                            auth.get_access_token().await,
                            Err(AuthError::AuthorizationRequired)
                        ) || oauth::is_auth_required(&message)
                    }
                    None => false,
                };
                Ok(text_result(
                    if signed_out {
                        oauth::sign_in_message(&route.server)
                    } else {
                        format!("{}: {message}", route.server)
                    },
                    true,
                ))
            }
        }
    }
}

/// Serve the hub on this process's stdin and stdout until the client disconnects. Diagnostics go
/// to stderr; stdout carries the protocol only.
pub fn run_stdio(config: HubConfig, store: Arc<dyn SecretStore>, state: PathBuf) -> Result<()> {
    use rmcp::ServiceExt;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| HubError::Invalid(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let server = HubServer::new(config, store, state)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| HubError::Invalid(format!("cannot start the hub: {e}")))?;
        server
            .waiting()
            .await
            .map_err(|e| HubError::Invalid(format!("the hub stopped: {e}")))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitised_and_capped() {
        assert_eq!(exposed_name("gh", "list_repos"), "gh__list_repos");
        assert_eq!(exposed_name("gh", "a.b c/d"), "gh__a_b_c_d");
        assert_eq!(exposed_name("gh", "é"), "gh___");
        let long = exposed_name("server", &"x".repeat(100));
        assert_eq!(long.len(), 64);
        assert!(long.starts_with("server__x"));
    }

    #[test]
    fn colliding_names_get_a_suffix_within_the_cap() {
        let mut taken = BTreeSet::new();
        let first = unique_name("s", "a.b", &taken);
        taken.insert(first.clone());
        let second = unique_name("s", "a_b", &taken);
        assert_eq!((first.as_str(), second.as_str()), ("s__a_b", "s__a_b_2"));
        let long = "x".repeat(100);
        let a = unique_name("s", &long, &taken);
        taken.insert(a.clone());
        let b = unique_name("s", &long, &taken);
        assert_eq!(a.len(), 64);
        assert_eq!(b.len(), 64);
        assert!(b.ends_with("_2"));
        assert_ne!(a, b);
    }
}
