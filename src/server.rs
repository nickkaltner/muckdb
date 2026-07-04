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
use axum::routing::{get, post};
use axum::{Router, http::StatusCode, http::header};
use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;

use crate::facade::PORT;
use crate::{introspect, paths, session, store};

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
        .route("/api/predict", get(api_predict))
        .route("/api/junk", get(api_junk))
        .route("/api/facets", get(api_facets))
        .route("/api/export", get(api_export))
        .route("/api/schema", get(api_schema))
        .route("/api/formats", get(api_formats))
        .route("/api/query", get(api_query))
        .route("/api/sessions", get(api_sessions))
        .route("/api/session", get(api_session))
        .route("/api/session/export", get(api_session_export))
        // Axum's default body limit is 2 MB — far too small for archives that
        // carry full database snapshots.
        .route(
            "/api/session/import",
            post(api_session_import)
                .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024 * 1024)),
        )
        .route("/api/shot", get(api_shot))
        .route("/api/session/rm", post(api_session_rm))
        .route("/api/forget", post(api_forget))
        .route("/api/trash", post(api_trash))
        .route("/api/activity", post(api_activity))
        .route("/chart.js", get(chart_js))
        .route("/chart-adapter.js", get(chart_adapter_js))
        .route("/ws", get(ws_handler))
        // SPA fallback: client-routed paths like /db/<name>/<table> serve the app.
        .fallback(get(index))
        .with_state(state);

    // Loopback by default — the console exposes every database muckdb has
    // touched, so it shouldn't be on the LAN unless deliberately opened up
    // (MUCKDB_BIND=0.0.0.0, or any specific interface address).
    let bind: std::net::IpAddr = std::env::var("MUCKDB_BIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::net::IpAddr::from([127, 0, 0, 1]));
    let addr = SocketAddr::from((bind, PORT));
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

/// The full snapshot pushed to browsers: history + databases + session summaries.
fn snapshot_json() -> Option<String> {
    let state = store::load_state().ok()?;
    let sessions: Vec<_> = session::list()
        .unwrap_or_default()
        .into_iter()
        .map(|s| json!({ "id": s.id, "title": s.title, "updated": s.updated, "tiles": s.tiles.len() }))
        .collect();
    serde_json::to_string(&json!({
        "history": state.history,
        "databases": state.databases,
        "sessions": sessions,
        // Fingerprint of the column-format registry: a `muckdb format` write
        // lands in the watched data dir, and this field is what makes the
        // snapshot differ so the change is actually broadcast.
        "formats_rev": crate::formats::registry_rev(),
    }))
    .ok()
}

/// Watch the history store and session files; broadcast fresh state on changes.
fn spawn_watcher(tx: broadcast::Sender<String>) -> Result<()> {
    let data_dir = paths::data_dir()?;
    let sessions_dir = session::sessions_dir()?;
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
    watcher
        .watch(&sessions_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {sessions_dir:?}"))?;

    thread::spawn(move || {
        // The watcher is moved in so it lives as long as this thread.
        let _watcher = watcher;
        // Dedupe: only broadcast when the snapshot actually changed, so a stray
        // filesystem event never turns into a client-side refresh storm.
        let mut last_sent = String::new();
        for _ in raw_rx {
            if let Some(s) = snapshot_json()
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
    // The app is a static asset except for the build version (shown in the
    // credits card) — stamp it once and serve the cached result.
    static PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        include_str!("assets/index.html").replace("__MUCKDB_VERSION__", env!("CARGO_PKG_VERSION"))
    });
    Html(PAGE.as_str())
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
                    json!({ "id": store::db_id(&p), "path": p, "exists": exists })
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
    sort: Option<String>,
    dir: Option<String>,
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
    match introspect::preview(
        &p.db,
        &p.table,
        limit,
        offset,
        p.q.as_deref(),
        &filters,
        p.sort.as_deref(),
        p.dir.as_deref(),
    ) {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct StatsParams {
    db: String,
    table: String,
    // Active search + facet filters (ignored by schema, which reuses this type).
    q: Option<String>,
    filter: Option<String>,
}

async fn api_stats(Query(p): Query<StatsParams>) -> Response {
    let filters = parse_filters(p.filter.as_deref());
    match introspect::stats(&p.db, &p.table, p.q.as_deref(), &filters) {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => error_json(&e),
    }
}

/// Pairwise prediction matrix — heavier than stats (one big duckdb script),
/// so it runs on the blocking pool instead of a tokio worker.
async fn api_predict(Query(p): Query<StatsParams>) -> Response {
    let filters = parse_filters(p.filter.as_deref());
    let result = tokio::task::spawn_blocking(move || {
        crate::predict::predict(&p.db, &p.table, p.q.as_deref(), &filters)
    })
    .await;
    match result {
        Ok(Ok(pred)) => Json(pred).into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("predict task failed: {e}")),
    }
}

/// Column-health metrics for the junk-data tab (same params as /api/stats).
async fn api_junk(Query(p): Query<StatsParams>) -> Response {
    let filters = parse_filters(p.filter.as_deref());
    let result = tokio::task::spawn_blocking(move || {
        crate::predict::junk(&p.db, &p.table, p.q.as_deref(), &filters)
    })
    .await;
    match result {
        Ok(Ok(j)) => Json(j).into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("junk task failed: {e}")),
    }
}

#[derive(Deserialize)]
struct FacetsParams {
    db: String,
    table: String,
    q: Option<String>,
    filter: Option<String>,
}

#[derive(Deserialize)]
struct DbParam {
    db: String,
}

/// Merged column display formats (comments + registry) for a database.
async fn api_formats(Query(p): Query<DbParam>) -> Response {
    Json(crate::formats::merged_for(&p.db)).into_response()
}

async fn api_facets(Query(p): Query<FacetsParams>) -> Response {
    let filters = parse_filters(p.filter.as_deref());
    match introspect::facets(&p.db, &p.table, p.q.as_deref(), &filters) {
        Ok(facets) => Json(json!({ "facets": facets })).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct ExportParams {
    db: String,
    table: String,
    format: Option<String>,
    q: Option<String>,
    filter: Option<String>,
    /// JSON array of column names to exclude (facet eyes the user closed).
    hide: Option<String>,
}

/// Keep a table name safe to drop into a Content-Disposition filename.
fn safe_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

async fn api_export(Query(p): Query<ExportParams>) -> Response {
    let fmt = p.format.as_deref().unwrap_or("csv").to_ascii_lowercase();
    let filters = parse_filters(p.filter.as_deref());
    let hidden: Vec<String> = p
        .hide
        .as_deref()
        .and_then(|h| serde_json::from_str(h).ok())
        .unwrap_or_default();
    match introspect::export(&p.db, &p.table, &fmt, p.q.as_deref(), &filters, &hidden) {
        Ok(body) => {
            let (ctype, ext) = if fmt == "json" {
                ("application/json", "json")
            } else {
                ("text/csv", "csv")
            };
            let disposition = format!(
                "attachment; filename=\"{}.{}\"",
                safe_filename(&p.table),
                ext
            );
            (
                [
                    (header::CONTENT_TYPE, ctype.to_string()),
                    (header::CONTENT_DISPOSITION, disposition),
                ],
                body,
            )
                .into_response()
        }
        Err(e) => error_json(&e),
    }
}

async fn api_schema(Query(p): Query<StatsParams>) -> Response {
    match introspect::schema(&p.db, &p.table) {
        Ok(cols) => {
            let sql = introspect::view_sql(&p.db, &p.table).ok().flatten();
            Json(json!({ "columns": cols, "sql": sql })).into_response()
        }
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct QueryParams {
    db: String,
    sql: String,
}

async fn api_query(Query(p): Query<QueryParams>) -> Response {
    match introspect::query(&p.db, &p.sql) {
        Ok(result) => Json(result).into_response(),
        Err(e) => error_json(&e),
    }
}

async fn api_sessions() -> Response {
    match session::list() {
        Ok(sessions) => {
            let out: Vec<_> = sessions
                .into_iter()
                .map(|s| json!({ "id": s.id, "title": s.title, "updated": s.updated, "tiles": s.tiles.len() }))
                .collect();
            Json(json!({ "sessions": out })).into_response()
        }
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct SessionParams {
    id: String,
}

async fn api_session(Query(p): Query<SessionParams>) -> Response {
    match session::load(&p.id) {
        Ok(Some(s)) => Json(s).into_response(),
        Ok(None) => (StatusCode::OK, Json(json!({ "error": "no such session" }))).into_response(),
        Err(e) => error_json(&e),
    }
}

/// Bundle a session into a `.muckdb` zip download (the export button).
async fn api_session_export(Query(p): Query<SessionParams>) -> Response {
    let id = session::slug(&p.id);
    let result =
        tokio::task::spawn_blocking(move || crate::export::export_session_bytes(&id)).await;
    match result {
        Ok(Ok((name, bytes))) => (
            [
                (header::CONTENT_TYPE, "application/zip".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}.muckdb\"", safe_filename(&name)),
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("export task failed: {e}")),
    }
}

/// Install an uploaded `.muckdb` archive (the header import button). The
/// session save lands in the watched sessions dir, so every viewer's session
/// list updates on its own.
async fn api_session_import(body: axum::body::Bytes) -> Response {
    let result =
        tokio::task::spawn_blocking(move || crate::export::import_and_install(&body)).await;
    match result {
        Ok(Ok(imported)) => Json(json!({ "ok": true, "id": imported.session.id })).into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("import task failed: {e}")),
    }
}

/// Delete a session (the dashboard's armed delete button). The watcher sees
/// the file vanish and refreshes every viewer's session list.
async fn api_session_rm(Query(p): Query<SessionParams>) -> Response {
    let id = session::slug(&p.id);
    match session::remove(&id) {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error_json(&anyhow::anyhow!("no such session '{id}'")),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct ForgetParams {
    db: String,
}

/// Append a "forget this database" tombstone (the error card's "remove this
/// db" button). Using the db again resurfaces it; the history watcher pushes
/// the shrunken list to every viewer.
async fn api_forget(Query(p): Query<ForgetParams>) -> Response {
    match store::forget_db(&p.db) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct TrashParams {
    session: String,
    tile: String,
    /// "1" to trash, "0" to restore.
    on: String,
}

/// The one write endpoint: flip a tile's trashed flag in the session JSON.
/// The file watcher sees the save and pushes the update to every viewer.
async fn api_trash(Query(p): Query<TrashParams>) -> Response {
    let on = p.on != "0";
    match session::set_tile_trashed(&p.session, &p.tile, on) {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => (
            StatusCode::OK,
            Json(json!({ "error": "no such session or tile" })),
        )
            .into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct ActivityParams {
    session: String,
    tile: Option<String>,
    /// "view" (no tile) | "zoom" | "explore".
    #[serde(default)]
    action: String,
}

/// Record a human interaction (session open, panel zoom, explore click) in
/// activity.json — read back by `muckdb ls session(s)` so agents can see what
/// the human actually looks at. Writes outside the watched sessions dir, so
/// this never triggers a re-render.
async fn api_activity(Query(p): Query<ActivityParams>) -> Response {
    match session::record_activity(&p.session, p.tile.as_deref(), &p.action) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_json(&e),
    }
}

#[derive(Deserialize)]
struct ShotParams {
    session: String,
    tile: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

/// Render a session (or one tile) to PNG via a local headless Chromium — the
/// backend for the copy-image button and for `curl`-able panel captures.
async fn api_shot(Query(p): Query<ShotParams>) -> Response {
    // Validate up front so a bad name is a JSON error, not a PNG of one.
    let id = session::slug(&p.session);
    match session::load(&id) {
        Ok(Some(s)) => {
            if let Some(t) = &p.tile
                && !s.tiles.iter().any(|x| x.name() == t.as_str())
            {
                return error_json(&anyhow::anyhow!("no tile '{t}' in session {id}"));
            }
        }
        Ok(None) => return error_json(&anyhow::anyhow!("no such session '{id}'")),
        Err(e) => return error_json(&e),
    }
    // One capture at a time — each spawns a browser.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    let width = p.width.unwrap_or(crate::shot::DEFAULT_WIDTH);
    let result = tokio::task::spawn_blocking(move || {
        crate::shot::capture_png(&id, p.tile.as_deref(), width, p.height)
    })
    .await;
    match result {
        Ok(Ok(png)) => ([(header::CONTENT_TYPE, "image/png")], png).into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("screenshot task failed: {e}")),
    }
}

/// Serve the vendored Chart.js for the session dashboards.
async fn chart_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("assets/chart.umd.min.js"),
    )
        .into_response()
}

/// Serve the vendored Chart.js date adapter (enables time-axis charts).
async fn chart_adapter_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("assets/chart-adapter.js"),
    )
        .into_response()
}

fn error_json(e: &anyhow::Error) -> Response {
    (StatusCode::OK, Json(json!({ "error": format!("{e:#}") }))).into_response()
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Send the current snapshot immediately so a fresh client is populated.
    if let Some(s) = snapshot_json()
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
