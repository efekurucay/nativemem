//! A fake OAuth authorization server and MCP HTTP server in one, on 127.0.0.1. It speaks just
//! enough HTTP for the hub: protected resource metadata, authorization server metadata, dynamic
//! client registration, authorization with PKCE (a redirect), token exchange and refresh, and an
//! MCP endpoint that wants a bearer token. Shared by `crates/hub/tests/oauth.rs` and
//! `crates/cli/tests/mcp.rs`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct State {
    /// Access tokens the MCP endpoint accepts.
    valid_access: Vec<String>,
    refresh_token: Option<String>,
    fail_refresh: bool,
    /// Authorization codes and the PKCE challenge they were issued for.
    codes: HashMap<String, String>,
    clients: Vec<(String, String)>,
    issued: usize,
    registrations: usize,
    refreshes: usize,
    authorizations: usize,
    challenges: usize,
    bearers: Vec<String>,
    token_requests: Vec<String>,
}

#[derive(Clone)]
pub struct FakeOauth {
    pub base: String,
    state: Arc<Mutex<State>>,
}

struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

impl FakeOauth {
    pub async fn start() -> FakeOauth {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let fake = FakeOauth {
            base: format!("http://127.0.0.1:{port}"),
            state: Arc::default(),
        };
        let serving = fake.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let serving = serving.clone();
                tokio::spawn(async move { serving.serve(socket).await });
            }
        });
        fake
    }

    /// The URL of the MCP endpoint.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base)
    }

    /// The access tokens stop working; the refresh token stays valid.
    pub fn revoke_access(&self) {
        self.state.lock().unwrap().valid_access.clear();
    }

    /// Refreshing answers `invalid_grant`.
    pub fn fail_refresh(&self) {
        self.state.lock().unwrap().fail_refresh = true;
    }

    pub fn registrations(&self) -> usize {
        self.state.lock().unwrap().registrations
    }

    pub fn refreshes(&self) -> usize {
        self.state.lock().unwrap().refreshes
    }

    pub fn authorizations(&self) -> usize {
        self.state.lock().unwrap().authorizations
    }

    /// 401 answers that carried a `resource_metadata` challenge.
    pub fn challenges(&self) -> usize {
        self.state.lock().unwrap().challenges
    }

    /// Every bearer token the MCP endpoint saw, in order.
    pub fn bearers(&self) -> Vec<String> {
        self.state.lock().unwrap().bearers.clone()
    }

    /// The form bodies the token endpoint received.
    pub fn token_requests(&self) -> Vec<String> {
        self.state.lock().unwrap().token_requests.clone()
    }

    async fn serve(&self, mut socket: TcpStream) {
        let Some(request) = read_request(&mut socket).await else {
            return;
        };
        let (status, headers, body) = self.route(&request);
        let mut head = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
            body.len()
        );
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(body.as_bytes()).await;
    }

    fn route(&self, request: &Request) -> (&'static str, Vec<(String, String)>, String) {
        let (path, query) = request
            .target
            .split_once('?')
            .unwrap_or((request.target.as_str(), ""));
        let json_header = || vec![("content-type".to_string(), "application/json".to_string())];
        let reply = |status, value: Value| (status, json_header(), value.to_string());
        let base = &self.base;
        match (request.method.as_str(), path) {
            ("GET", p) if p.starts_with("/.well-known/oauth-protected-resource") => reply(
                "200 OK",
                json!({ "resource": format!("{base}/mcp"), "authorization_servers": [base] }),
            ),
            ("GET", p) if p.starts_with("/.well-known/oauth-authorization-server") => reply(
                "200 OK",
                json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "registration_endpoint": format!("{base}/register"),
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "code_challenge_methods_supported": ["S256"],
                    "token_endpoint_auth_methods_supported": ["none"],
                }),
            ),
            ("POST", "/register") => {
                let doc: Value = serde_json::from_str(&request.body).unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                state.registrations += 1;
                let client_id = format!("client-{}", state.registrations);
                for uri in doc["redirect_uris"].as_array().into_iter().flatten() {
                    state.clients.push((
                        client_id.clone(),
                        uri.as_str().unwrap_or_default().to_string(),
                    ));
                }
                reply(
                    "201 Created",
                    json!({ "client_id": client_id, "redirect_uris": doc["redirect_uris"] }),
                )
            }
            ("GET", "/authorize") => self.authorize(query),
            ("POST", "/token") => self.token(&request.body),
            ("POST", "/mcp") => self.mcp(request),
            _ => ("404 Not Found", Vec::new(), String::new()),
        }
    }

    fn authorize(&self, query: &str) -> (&'static str, Vec<(String, String)>, String) {
        let params = form(query);
        let get = |k: &str| params.get(k).cloned().unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        let known = state
            .clients
            .iter()
            .any(|(id, uri)| *id == get("client_id") && *uri == get("redirect_uri"));
        if !known || get("code_challenge_method") != "S256" || get("code_challenge").is_empty() {
            return (
                "400 Bad Request",
                Vec::new(),
                "unknown client or redirect".into(),
            );
        }
        state.authorizations += 1;
        state.issued += 1;
        let code = format!("code-{}", state.issued);
        state.codes.insert(code.clone(), get("code_challenge"));
        let location = format!("{}?code={code}&state={}", get("redirect_uri"), get("state"));
        (
            "302 Found",
            vec![("location".to_string(), location)],
            String::new(),
        )
    }

    fn token(&self, body: &str) -> (&'static str, Vec<(String, String)>, String) {
        let params = form(body);
        let get = |k: &str| params.get(k).cloned().unwrap_or_default();
        let json_header = vec![("content-type".to_string(), "application/json".to_string())];
        let mut state = self.state.lock().unwrap();
        state.token_requests.push(body.to_string());
        let bad = |error: &str| {
            (
                "400 Bad Request",
                vec![("content-type".to_string(), "application/json".to_string())],
                json!({ "error": error }).to_string(),
            )
        };
        match get("grant_type").as_str() {
            "authorization_code" => {
                let Some(challenge) = state.codes.remove(&get("code")) else {
                    return bad("invalid_grant");
                };
                let digest = Sha256::digest(get("code_verifier").as_bytes());
                let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
                if expected != challenge {
                    return bad("invalid_grant");
                }
            }
            "refresh_token" => {
                if state.fail_refresh
                    || state.refresh_token.as_deref() != Some(&get("refresh_token"))
                {
                    return bad("invalid_grant");
                }
                state.refreshes += 1;
            }
            _ => return bad("unsupported_grant_type"),
        }
        state.issued += 1;
        let access = format!("at-{}", state.issued);
        let refresh = format!("rt-{}", state.issued);
        state.valid_access.push(access.clone());
        state.refresh_token = Some(refresh.clone());
        (
            "200 OK",
            json_header,
            json!({
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token": refresh,
            })
            .to_string(),
        )
    }

    fn mcp(&self, request: &Request) -> (&'static str, Vec<(String, String)>, String) {
        let bearer = request
            .headers
            .get("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_string);
        let mut state = self.state.lock().unwrap();
        if let Some(token) = &bearer {
            state.bearers.push(token.clone());
        }
        if !bearer.is_some_and(|t| state.valid_access.contains(&t)) {
            state.challenges += 1;
            let challenge = format!(
                "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
                self.base
            );
            return (
                "401 Unauthorized",
                vec![("www-authenticate".to_string(), challenge)],
                String::new(),
            );
        }
        let doc: Value = serde_json::from_str(&request.body).unwrap_or_default();
        let id = doc["id"].clone();
        let json_header = vec![("content-type".to_string(), "application/json".to_string())];
        let result = match doc["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-oauth", "version": "0" },
            }),
            Some("tools/list") => json!({ "tools": [{
                "name": "whoami",
                "description": "Says who is calling",
                "inputSchema": { "type": "object" },
            }] }),
            Some("tools/call") => json!({ "content": [{ "type": "text", "text": "signed in" }] }),
            _ => return ("202 Accepted", Vec::new(), String::new()),
        };
        (
            "200 OK",
            json_header,
            json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        )
    }
}

fn form(text: &str) -> HashMap<String, String> {
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect()
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn read_request(socket: &mut TcpStream) -> Option<Request> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head_end, length) = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..pos]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")?
                        .trim()
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(0);
            break (pos + 4, length);
        }
    };
    while data.len() < head_end + length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let body = String::from_utf8_lossy(&data[head_end..]).into_owned();
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}
