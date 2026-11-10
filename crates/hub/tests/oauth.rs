//! OAuth sign-in against a fake authorization server and MCP server on 127.0.0.1 (no real
//! network). The browser is replaced by a function that follows the authorization URL itself.
//! Secrets live in a file store inside a temp directory.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::fake_oauth::FakeOauth;
use nativemem_hub::config::{Auth, HubConfig, ServerSpec};
use nativemem_hub::connect::{connect, ConnectError};
use nativemem_hub::oauth::{
    self, client_secret_id, login, logout, tokens_secret_id, AuthState, LoginOptions,
};
use nativemem_hub::secrets::{FileStore, SecretStore};
use nativemem_hub::serve::HubServer;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::ServiceExt;
use serde_json::{json, Value};

fn spec(fake: &FakeOauth) -> ServerSpec {
    let mut spec = ServerSpec::http(fake.mcp_url());
    spec.auth = Auth::Oauth;
    spec
}

fn store(dir: &Path) -> Arc<dyn SecretStore> {
    Arc::new(FileStore::new(dir))
}

/// A browser that follows the authorization URL, redirects included, like a signed-in user would.
fn browser(url: &str) {
    let url = url.to_string();
    tokio::spawn(async move {
        let _ = reqwest::get(url).await;
    });
}

async fn sign_in(name: &str, spec: &ServerSpec, store: &Arc<dyn SecretStore>, state: &Path) {
    let (name, spec, store, state) = (
        name.to_string(),
        spec.clone(),
        store.clone(),
        state.to_path_buf(),
    );
    tokio::task::spawn_blocking(move || {
        login(
            &name,
            &spec,
            store,
            &state,
            &LoginOptions {
                timeout: Duration::from_secs(30),
                open: &browser,
            },
        )
    })
    .await
    .unwrap()
    .unwrap_or_else(|e| panic!("login failed: {e}"));
}

/// The stored token record with its receive time set far in the past: the access token is expired.
fn expire_stored_token(store: &dyn SecretStore, name: &str) {
    let id = tokens_secret_id(name);
    let mut doc: Value = serde_json::from_str(&store.get(&id).unwrap().unwrap()).unwrap();
    doc["token_received_at"] = json!(1);
    store.set(&id, &doc.to_string()).unwrap();
}

fn stored_json(store: &dyn SecretStore, id: &str) -> Value {
    serde_json::from_str(&store.get(id).unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn login_discovers_registers_stores_and_the_bearer_reaches_the_upstream() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    assert_eq!(
        oauth::auth_state("fake", &spec, secrets.as_ref()).unwrap(),
        AuthState::NeedsLogin
    );

    sign_in("fake", &spec, &secrets, t.path()).await;

    assert_eq!(fake.registrations(), 1);
    assert_eq!(fake.authorizations(), 1);
    assert!(
        fake.challenges() >= 1,
        "discovery starts from the 401 challenge"
    );
    assert_eq!(
        oauth::auth_state("fake", &spec, secrets.as_ref()).unwrap(),
        AuthState::SignedIn
    );
    let client = stored_json(secrets.as_ref(), &client_secret_id("fake"));
    assert_eq!(client["client_id"], "client-1");
    assert!(client["redirect_uri"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    let tokens = stored_json(secrets.as_ref(), &tokens_secret_id("fake"));
    assert!(tokens["token_response"]["access_token"].is_string());
    assert!(tokens["token_response"]["refresh_token"].is_string());
    assert_eq!(tokens["token_response"]["expires_in"], 3600);

    let connected = connect("fake", &spec, secrets.clone(), t.path())
        .await
        .unwrap_or_else(|e| panic!("connect failed: {e}"));
    let tools: Vec<&str> = connected.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(tools, ["whoami"]);
    let access = tokens["token_response"]["access_token"].as_str().unwrap();
    assert!(fake.bearers().iter().any(|b| b == access));
    assert_eq!(fake.refreshes(), 0);
}

#[tokio::test]
async fn a_second_login_reuses_the_registered_client() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    let first = stored_json(secrets.as_ref(), &client_secret_id("fake"));
    sign_in("fake", &spec, &secrets, t.path()).await;
    assert_eq!(fake.registrations(), 1);
    assert_eq!(fake.authorizations(), 2);
    assert_eq!(
        stored_json(secrets.as_ref(), &client_secret_id("fake")),
        first
    );
}

#[tokio::test]
async fn a_taken_loopback_port_registers_a_new_client() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    let first = stored_json(secrets.as_ref(), &client_secret_id("fake"));
    let port: u16 = first["redirect_uri"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/callback")
        .parse()
        .unwrap();
    let _taken = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();

    sign_in("fake", &spec, &secrets, t.path()).await;
    assert_eq!(fake.registrations(), 2);
    let second = stored_json(secrets.as_ref(), &client_secret_id("fake"));
    assert_eq!(second["client_id"], "client-2");
    assert_ne!(second["redirect_uri"], first["redirect_uri"]);
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_the_new_one_reaches_the_upstream() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    let before = stored_json(secrets.as_ref(), &tokens_secret_id("fake"));
    expire_stored_token(secrets.as_ref(), "fake");

    let connected = connect("fake", &spec, secrets.clone(), t.path())
        .await
        .unwrap_or_else(|e| panic!("connect failed: {e}"));
    assert_eq!(connected.tools.len(), 1);
    assert_eq!(fake.refreshes(), 1);
    let after = stored_json(secrets.as_ref(), &tokens_secret_id("fake"));
    let new_access = after["token_response"]["access_token"].as_str().unwrap();
    assert_ne!(
        new_access,
        before["token_response"]["access_token"].as_str().unwrap()
    );
    assert_eq!(fake.bearers().last().map(String::as_str), Some(new_access));
    // The refresh request carries the refresh token the store held after taking the lock.
    let last = fake.token_requests().pop().unwrap();
    assert!(last.contains("grant_type=refresh_token"), "{last}");
    assert!(last.contains("resource="), "{last}");
}

#[tokio::test]
async fn a_token_the_server_rejects_is_refreshed_and_the_call_retried() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    let connected = connect("fake", &spec, secrets.clone(), t.path())
        .await
        .unwrap();

    fake.revoke_access();
    let result = connected
        .client
        .peer()
        .call_tool(CallToolRequestParams::new("whoami".to_string()))
        .await
        .expect("the call survives a silent refresh");
    assert_eq!(
        result.content[0].as_text().map(|t| t.text.as_str()),
        Some("signed in")
    );
    assert_eq!(fake.refreshes(), 1);
}

#[tokio::test]
async fn a_failed_refresh_needs_a_new_login() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    expire_stored_token(secrets.as_ref(), "fake");
    fake.fail_refresh();

    let Err(error) = connect("fake", &spec, secrets.clone(), t.path()).await else {
        panic!("the connection must fail");
    };
    assert_eq!(error, ConnectError::NeedsLogin("fake".into()));
    assert_eq!(error.to_string(), "Sign in again: nativemem mcp login fake");
    // The rejected tokens are gone, so the next start does not even try the network.
    assert_eq!(
        oauth::auth_state("fake", &spec, secrets.as_ref()).unwrap(),
        AuthState::NeedsLogin
    );
    assert!(secrets.get(&client_secret_id("fake")).unwrap().is_some());
}

#[tokio::test]
async fn the_hub_reports_needs_login_and_its_tools_answer_with_the_sign_in_message() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;

    let mut config = HubConfig::default();
    config.servers.insert("fake".into(), spec);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (store_for_hub, state) = (secrets.clone(), t.path().to_path_buf());
    tokio::spawn(async move {
        if let Ok(server) = HubServer::new(config, store_for_hub, state)
            .serve(server_io)
            .await
        {
            let _ = server.waiting().await;
        }
    });
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(client_io)
    .await
    .unwrap();

    // Signed in: the upstream tool works.
    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["nativemem_status", "fake__whoami"]);

    // The sign-in stops working during the session.
    fake.revoke_access();
    fake.fail_refresh();
    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new("fake__whoami".to_string()))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.content[0].as_text().map(|t| t.text.as_str()),
        Some("Sign in again: nativemem mcp login fake")
    );
}

#[tokio::test]
async fn a_server_that_starts_signed_out_exposes_the_sign_in_tool() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let mut config = HubConfig::default();
    config.servers.insert("fake".into(), spec(&fake));
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (store_for_hub, state) = (secrets.clone(), t.path().to_path_buf());
    tokio::spawn(async move {
        if let Ok(server) = HubServer::new(config, store_for_hub, state)
            .serve(server_io)
            .await
        {
            let _ = server.waiting().await;
        }
    });
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(client_io)
    .await
    .unwrap();

    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["nativemem_status", "fake__sign_in_required"]);
    let status = client
        .peer()
        .call_tool(CallToolRequestParams::new("nativemem_status".to_string()))
        .await
        .unwrap();
    let doc: Value = serde_json::from_str(&status.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(doc["servers"][0]["status"], "needs_login");
    assert_eq!(
        doc["servers"][0]["error"],
        "Sign in again: nativemem mcp login fake"
    );
    let call = client
        .peer()
        .call_tool(CallToolRequestParams::new(
            "fake__sign_in_required".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(call.is_error, Some(true));
    assert_eq!(
        call.content[0].as_text().map(|t| t.text.as_str()),
        Some("Sign in again: nativemem mcp login fake")
    );
    assert_eq!(fake.bearers().len(), 0, "no token, no request");
}

#[tokio::test]
async fn logout_deletes_the_tokens_and_keeps_the_client() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;

    assert!(logout("fake", secrets.as_ref(), t.path()).unwrap());
    assert!(secrets.get(&tokens_secret_id("fake")).unwrap().is_none());
    assert!(secrets.get(&client_secret_id("fake")).unwrap().is_some());
    assert!(!logout("fake", secrets.as_ref(), t.path()).unwrap());
    assert_eq!(
        oauth::auth_state("fake", &spec, secrets.as_ref()).unwrap(),
        AuthState::NeedsLogin
    );
}

#[tokio::test]
async fn login_gives_up_when_the_browser_never_comes_back() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let (spec, store_for_login, state) = (spec(&fake), secrets.clone(), t.path().to_path_buf());
    let error = tokio::task::spawn_blocking(move || {
        login(
            "fake",
            &spec,
            store_for_login,
            &state,
            &LoginOptions {
                timeout: Duration::from_millis(300),
                open: &|_| {},
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("no sign-in within"), "{error}");
    assert!(secrets.get(&tokens_secret_id("fake")).unwrap().is_none());
}

#[tokio::test]
async fn a_refused_authorization_is_reported_without_free_text() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let (spec, store_for_login, state) = (spec(&fake), secrets.clone(), t.path().to_path_buf());
    let error = tokio::task::spawn_blocking(move || {
        login(
            "fake",
            &spec,
            store_for_login,
            &state,
            &LoginOptions {
                timeout: Duration::from_secs(30),
                open: &|url| {
                    let url = reqwest::Url::parse(url).unwrap();
                    let redirect = url
                        .query_pairs()
                        .find(|(k, _)| k == "redirect_uri")
                        .unwrap()
                        .1
                        .into_owned();
                    tokio::spawn(async move {
                        let _ = reqwest::get(format!(
                            "{redirect}?error=access_denied&error_description=secret+words"
                        ))
                        .await;
                    });
                },
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("access_denied"), "{text}");
    assert!(!text.contains("secret words"), "{text}");
}

#[tokio::test]
async fn login_refuses_a_server_without_oauth() {
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let (state, store_for_login) = (t.path().to_path_buf(), secrets.clone());
    let error = tokio::task::spawn_blocking(move || {
        login(
            "plain",
            &ServerSpec::http("http://127.0.0.1:1/mcp"),
            store_for_login,
            &state,
            &LoginOptions {
                timeout: Duration::from_secs(1),
                open: &|_| {},
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("oauth"), "{error}");
}

#[tokio::test]
async fn a_refresh_waits_for_the_lock_and_reads_the_store_again() {
    let fake = FakeOauth::start().await;
    let t = tempfile::tempdir().unwrap();
    let secrets = store(t.path());
    let spec = spec(&fake);
    sign_in("fake", &spec, &secrets, t.path()).await;
    expire_stored_token(secrets.as_ref(), "fake");
    connect("fake", &spec, secrets.clone(), t.path())
        .await
        .unwrap();
    assert_eq!(fake.refreshes(), 1);

    // This process sees an expired token with a refresh token the server no longer knows ...
    let id = tokens_secret_id("fake");
    let mut current = stored_json(secrets.as_ref(), &id);
    current["token_received_at"] = json!(1);
    let mut stale = current.clone();
    stale["token_response"]["refresh_token"] = json!("rotated-away");
    secrets.set(&id, &stale.to_string()).unwrap();

    // ... while another process holds the lock and then stores the current one.
    let lock = std::fs::File::options()
        .create(true)
        .append(true)
        .open(t.path().join(oauth::LOCK_FILE))
        .unwrap();
    lock.try_lock().unwrap();
    let (spec_again, store_again, state) = (spec.clone(), secrets.clone(), t.path().to_path_buf());
    let waiting = tokio::spawn(async move {
        connect("fake", &spec_again, store_again, &state)
            .await
            .map(|c| c.tools.len())
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!waiting.is_finished(), "the refresh waits for the lock");
    secrets.set(&id, &current.to_string()).unwrap();
    drop(lock);

    let tools = waiting.await.unwrap().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(tools, 1);
    assert_eq!(fake.refreshes(), 2);
}
