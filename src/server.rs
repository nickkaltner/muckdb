//! The daemon's HTTP + WebSocket server, its database-introspection API, mDNS
//! advertisement, and the file watcher that pushes live updates to browsers.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Router, http::StatusCode};
use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;

use crate::facade::PORT;
use crate::{introspect, paths, store};

const PREVIEW_LIMIT: u32 = 25;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
}

/// Entry point for the daemon: start mDNS, the file watcher, and the server.
pub async fn run() -> Result<()> {
    let (tx, _rx) = broadcast::channel::<String>(64);

    let _mdns = match register_mdns() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("muckdb: mDNS advertisement failed (continuing): {e:#}");
            None
        }
    };

    spawn_watcher(tx.clone())?;

    let state = AppState { tx };
    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/databases", get(api_databases))
        .route("/api/tables", get(api_tables))
        .route("/api/preview", get(api_preview))
        .route("/api/stats", get(api_stats))
        .route("/api/facets", get(api_facets))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    println!("muckdb daemon listening on http://localhost:{PORT}");
    axum::serve(listener, app).await.context("server error")?;
    Ok(())
}

/// Register `_muckdb._tcp.local.` over mDNS. The returned daemon must be kept
/// alive for the advertisement to persist.
fn register_mdns() -> Result<mdns_sd::ServiceDaemon> {
    use mdns_sd::{ServiceDaemon, ServiceInfo};
    let mdns = ServiceDaemon::new().context("creating mDNS daemon")?;
    let service = ServiceInfo::new(
        "_muckdb._tcp.local.",
        "muckdb",
        "muckdb.local.",
        "",
        PORT,
        &[("path", "/")][..],
    )
    .context("building mDNS service info")?
    .enable_addr_auto();
    mdns.register(service).context("registering mDNS service")?;
    Ok(mdns)
}

/// Watch the history store and broadcast fresh state on every change.
fn spawn_watcher(tx: broadcast::Sender<String>) -> Result<()> {
    let data_dir = paths::data_dir()?;
    let (raw_tx, raw_rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        // Only react to events that can change the store's *contents*. Reads of
        // the watched file (which load_state performs below) emit Access and
        // Metadata/atime events — reacting to those would feed back into the
        // watcher forever. Real appends emit Modify(Data).
        if let Ok(event) = res {
            let relevant = match event.kind {
                EventKind::Create(_) | EventKind::Remove(_) => true,
                EventKind::Modify(ModifyKind::Metadata(_)) => false,
                EventKind::Modify(_) => true,
                _ => false,
            };
            if relevant {
                let _ = raw_tx.send(());
            }
        }
    })
    .context("creating file watcher")?;
    watcher
        .watch(&data_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {data_dir:?}"))?;

    thread::spawn(move || {
        // The watcher is moved in so it lives as long as this thread.
        let _watcher = watcher;
        // Dedupe: only broadcast when the derived state actually changed, so a
        // stray filesystem event never turns into a client-side refresh storm.
        let mut last_sent = String::new();
        for _ in raw_rx {
            if let Ok(state) = store::load_state()
                && let Ok(s) = serde_json::to_string(&state)
                && s != last_sent
            {
                last_sent.clone_from(&s);
                let _ = tx.send(s);
            }
        }
    });
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("assets/index.html"))
}

/// Serialize the current derived state, or an error response.
fn state_response() -> Response {
    match store::load_state() {
        Ok(state) => Json(state).into_response(),
        Err(e) => error_json(&e),
    }
}

async fn api_state() -> Response {
    state_response()
}

async fn api_databases() -> Response {
    match store::load_state() {
        Ok(state) => {
            let dbs: Vec<_> = state
                .databases
                .into_iter()
                .map(|p| {
                    let exists = Path::new(&p).exists();
                    json!({ "path": p, "exists": exists })
                })
                .collect();
            Json(json!({ "databases": dbs })).into_response()
        }
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct TablesParams {
    db: String,
}

async fn api_tables(Query(p): Query<TablesParams>) -> Response {
    match introspect::list_tables(&p.db) {
        Ok(tables) => Json(json!({ "tables": tables })).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct PreviewParams {
    db: String,
    table: String,
    limit: Option<u32>,
    offset: Option<u32>,
    /// Free-text search across all columns.
    q: Option<String>,
    /// JSON array of `{ "column": .., "value": .. }` facet filters.
    filter: Option<String>,
}

/// Parse the JSON `filter` query param into facet filters (empty on absence/error).
fn parse_filters(raw: Option<&str>) -> Vec<introspect::Filter> {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

async fn api_preview(Query(p): Query<PreviewParams>) -> Response {
    let limit = p.limit.unwrap_or(PREVIEW_LIMIT).min(1000);
    let offset = p.offset.unwrap_or(0);
    let filters = parse_filters(p.filter.as_deref());
    match introspect::preview(&p.db, &p.table, limit, offset, p.q.as_deref(), &filters) {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct StatsParams {
    db: String,
    table: String,
}

async fn api_stats(Query(p): Query<StatsParams>) -> Response {
    match introspect::stats(&p.db, &p.table) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct FacetsParams {
    db: String,
    table: String,
    q: Option<String>,
    filter: Option<String>,
}

async fn api_facets(Query(p): Query<FacetsParams>) -> Response {
    let filters = parse_filters(p.filter.as_deref());
    match introspect::facets(&p.db, &p.table, p.q.as_deref(), &filters) {
        Ok(facets) => Json(json!({ "facets": facets })).into_response(),
        Err(e) => error_json(&e),
    }
}

fn error_json(e: &anyhow::Error) -> Response {
    (StatusCode::OK, Json(json!({ "error": format!("{e:#}") }))).into_response()
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Send the current snapshot immediately so a fresh client is populated.
    if let Ok(st) = store::load_state()
        && let Ok(s) = serde_json::to_string(&st)
        && socket.send(Message::Text(s.into())).await.is_err()
    {
        return;
    }

    let mut rx = state.tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(s) => {
                    if socket.send(Message::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => {}        // ignore client messages for now
                _ => break,              // closed or errored
            },
        }
    }
}
