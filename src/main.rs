//! git-lex-ui — a front door to every git-lex repo on the machine.
//!
//! Today, looking at a soul means knowing its path, cd-ing there, running
//! `git lex serve viz --port N`, and remembering which port went to which
//! soul. This starts once on a fixed port, reads the machine registry, drops
//! the entries that have rotted, shows what is left, and starts the right
//! server when you pick one.
//!
//! The browser only ever talks to this process. Per-repo servers are children
//! behind a proxy keyed on each repo's genesis sha — its permanent identity —
//! never on a port number, because a port number is a place and not a name.

mod layout;
mod sparql;
mod layout_api;
mod registry;
mod repo;
mod supervisor;

use axum::{
    Router,
    extract::{Path as AxPath, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use supervisor::Supervisor;
use tokio::sync::RwLock;

#[derive(Parser, Debug)]
#[command(
    name = "git-lex-ui",
    about = "A front door to every git-lex repo on the machine",
    version
)]
struct Args {
    /// Port for the front door itself.
    #[arg(long, default_value = "8888")]
    port: u16,

    /// First port to hand to per-repo servers. Each child may walk up to 20
    /// ports from where it is told to start, so allocations are spaced.
    #[arg(long, default_value = "7900")]
    repo_port_floor: u16,

    /// Read the registry but do not rewrite it. The registry is a shared,
    /// live file; this is the flag for when you want to look without
    /// touching.
    #[arg(long)]
    no_prune: bool,

    /// Do not open a browser on startup.
    #[arg(long)]
    no_open: bool,

    /// Directory holding the built frontend. Read from disk on every
    /// request, so it can be edited live.
    #[arg(long)]
    web: Option<PathBuf>,
}

struct AppState {
    repos: RwLock<Vec<repo::RepoProbe>>,
    /// genesis sha -> repo path. The proxy's routing table.
    by_genesis: RwLock<HashMap<String, String>>,
    prune: RwLock<Option<registry::PruneReport>>,
    dropped: RwLock<Vec<registry::Classified>>,
    sup: Arc<Supervisor>,
    http: reqwest::Client,
    web_dir: PathBuf,
    lex_dir: PathBuf,
    cache_dir: PathBuf,
    /// Packed layout payloads, by genesis sha, for the companion data route.
    layouts: RwLock<HashMap<String, Arc<Vec<u8>>>>,
    registry_path: RwLock<Option<String>>,
    registry_format: RwLock<Option<String>>,
    started_ms: u128,
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// A filesystem-safe, sortable stamp for the prune record's filename.
fn stamp() -> String {
    // Seconds since epoch. Deliberately not a formatted local time: this
    // names a file, and a name that depends on a timezone is a name that
    // changes meaning when the machine moves.
    format!("{}", now_ms() / 1000)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let home = home_dir();
    let lex_dir = home.join(".lex");
    let cache_dir = lex_dir.join("ui-cache");

    let web_dir = args.web.clone().unwrap_or_else(default_web_dir);

    let sup = Supervisor::new(args.repo_port_floor, &cache_dir);
    let state = Arc::new(AppState {
        repos: RwLock::new(vec![]),
        by_genesis: RwLock::new(HashMap::new()),
        prune: RwLock::new(None),
        dropped: RwLock::new(vec![]),
        sup: Arc::clone(&sup),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default(),
        web_dir: web_dir.clone(),
        lex_dir: lex_dir.clone(),
        cache_dir: cache_dir.clone(),
        layouts: RwLock::new(HashMap::new()),
        registry_path: RwLock::new(None),
        registry_format: RwLock::new(None),
        started_ms: now_ms(),
    });

    if let Err(e) = load_registry(&state, !args.no_prune).await {
        eprintln!("git-lex-ui: {e}");
        eprintln!("The front door still starts — you just get an empty list until this is fixed.");
    }

    // Take over anything a previous run left behind, before serving. Without
    // this, a supervisor that was killed rather than shut down starts a
    // second server for a repo that already has one, and the first keeps its
    // port forever.
    {
        let repos = state.repos.read().await.clone();
        let n = sup.adopt_existing(&repos).await;
        if n > 0 {
            println!("adopted {n} server(s) left running by a previous start");
        }
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/api/repos", get(api_repos))
        .route("/api/repos/reload", post(api_reload))
        .route("/api/servers", get(api_servers))
        .route("/api/servers/open", post(api_open))
        .route("/api/servers/stop", post(api_stop))
        .route("/api/health", get(api_health))
        .route("/api/layout/{genesis}", get(api_layout_meta))
        .route("/api/layout/{genesis}/data", get(api_layout_data))
        .route("/api/file/{genesis}", get(api_file))
        .route("/r/{genesis}/sparql", get(proxy_sparql).post(proxy_sparql))
        .route("/{*asset}", get(static_asset))
        .with_state(Arc::clone(&state));

    // Fixed port, and it fails loudly if taken. A front door that quietly
    // moves is a front door nobody can find — and this binary spends its
    // whole life defending against exactly that behaviour in its children,
    // so it had better not do it itself.
    let addr = format!("127.0.0.1:{}", args.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("git-lex-ui: could not bind {addr}: {e}");
            eprintln!(
                "Something already holds port {}. Stop it, or pass --port.",
                args.port
            );
            std::process::exit(1);
        }
    };

    let url = format!("http://{addr}");
    println!("git-lex-ui listening on {url}");
    println!("Frontend served from {}", web_dir.display());
    println!("Registry: {}", lex_dir.join("repos.*").display());
    println!("Per-repo data feed: git-lex-serve sparql, from {}", args.repo_port_floor);
    if !args.no_open {
        let _ = open::that_detached(&url);
    }

    // Health is asked on a timer, not remembered. A page left open for an
    // hour finds out its server died.
    let health_sup = Arc::clone(&sup);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            health_sup.refresh_health().await;
        }
    });

    let shutdown_sup = Arc::clone(&sup);
    let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nshutting down child servers…");
        shutdown_sup.stop_all().await;
    });
    if let Err(e) = serve.await {
        eprintln!("server error: {e}");
    }
}

fn default_web_dir() -> PathBuf {
    // Next to the binary in a release, or the source tree in development.
    let cwd = std::env::current_dir().unwrap_or_default();
    for c in [cwd.join("web/dist"), cwd.join("dist")] {
        if c.is_dir() {
            return c;
        }
    }
    cwd.join("web/dist")
}

/// Read the registry, classify every row, optionally prune, then probe what
/// survives.
async fn load_registry(state: &Arc<AppState>, do_prune: bool) -> Result<(), String> {
    let Some((path, fmt)) = registry::find_registry(&state.lex_dir) else {
        return Err(format!(
            "no registry found in {} (looked for repos.json, repos.yaml, repos.yml)",
            state.lex_dir.display()
        ));
    };
    *state.registry_path.write().await = Some(path.display().to_string());
    *state.registry_format.write().await = Some(
        match fmt {
            registry::Format::Json => "json",
            registry::Format::Yaml => "yaml",
        }
        .to_string(),
    );

    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let file = registry::parse(&text, fmt)?;
    let home = home_dir();
    let classified = registry::classify(&file.repos, &home);

    let (live, dead): (Vec<_>, Vec<_>) = classified
        .into_iter()
        .partition(|c| c.verdict.is_live());

    if do_prune && !dead.is_empty() {
        match registry::prune(&path, fmt, &dead, &state.cache_dir, &stamp()) {
            Ok(report) => {
                println!(
                    "registry: removed {} dead entries, kept {} (record: {})",
                    report.removed.len(),
                    report.kept,
                    report.backup_path.as_deref().unwrap_or("none")
                );
                *state.prune.write().await = Some(report);
            }
            Err(e) => eprintln!("registry: prune failed, leaving the file alone: {e}"),
        }
    }
    *state.dropped.write().await = dead;

    let rows: Vec<(PathBuf, Option<String>)> = live
        .iter()
        .map(|c| (PathBuf::from(&c.path), c.last_used.clone()))
        .collect();
    let mut probes = repo::probe_all(rows).await;

    // Most recent first. `recency_source` travels with every row so the
    // column can say which clock it used — 46 of 50 registry entries have no
    // `last_used`, so most of this ordering is commit time wearing a
    // borrowed hat, and the UI must not imply otherwise.
    probes.sort_by(|a, b| b.recency.cmp(&a.recency));

    let mut map = HashMap::new();
    for p in &probes {
        if let Some(g) = &p.genesis_sha {
            map.insert(g.clone(), p.path.clone());
        }
    }
    *state.by_genesis.write().await = map;
    *state.repos.write().await = probes;
    Ok(())
}

// ---------------------------------------------------------------- endpoints

#[derive(Serialize)]
struct ReposResponse {
    repos: Vec<repo::RepoProbe>,
    /// Everything the registry held that is not shown, and why. Disclose what
    /// you drop: a shorter list with no account of the difference is how a
    /// missing repo becomes an hour of confusion.
    dropped: Vec<registry::Classified>,
    prune: Option<registry::PruneReport>,
    registry_path: Option<String>,
    registry_format: Option<String>,
    counts: Counts,
}

#[derive(Serialize)]
struct Counts {
    shown: usize,
    dropped_total: usize,
    dropped_path_missing: usize,
    dropped_no_lex: usize,
    dropped_scratch: usize,
    /// How many shown rows are ordered by their own commit time because the
    /// registry had no `last_used` for them.
    recency_from_commit: usize,
}

async fn api_repos(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    // Re-read graph freshness and HEAD on every request rather than serving
    // the boot-time snapshot.
    //
    // This is not a nicety. The freshness marker exists to catch a graph that
    // has drifted behind its repo, and a marker computed once at startup
    // drifts in exactly the same way — it would sit there reading "current"
    // while the repo moved underneath it, which is the defect wearing the
    // costume of its own detector. Caught within ten minutes of shipping it,
    // by saving a file and watching the row not change.
    //
    // The cost is one `ls` plus one or two `git` calls per repo, run
    // concurrently. Everything expensive (layouts, the store) stays cached
    // and keyed by HEAD; this refreshes only the cheap facts that go stale.
    {
        let snapshot = s.repos.read().await.clone();
        let refreshed = futures_util::future::join_all(snapshot.into_iter().map(|r| async move {
            let last_used = if r.recency_source == repo::RecencySource::LastUsed {
                r.recency.clone()
            } else {
                None
            };
            repo::probe(std::path::Path::new(&r.path), last_used).await
        }))
        .await;
        *s.repos.write().await = refreshed;
    }
    let repos = s.repos.read().await.clone();
    let dropped = s.dropped.read().await.clone();
    let count_of = |v: registry::Verdict| dropped.iter().filter(|d| d.verdict == v).count();
    let counts = Counts {
        shown: repos.len(),
        dropped_total: dropped.len(),
        dropped_path_missing: count_of(registry::Verdict::PathMissing),
        dropped_no_lex: count_of(registry::Verdict::NoLexDir),
        dropped_scratch: count_of(registry::Verdict::Scratch),
        recency_from_commit: repos
            .iter()
            .filter(|r| r.recency_source == repo::RecencySource::HeadCommit)
            .count(),
    };
    axum::Json(ReposResponse {
        repos,
        dropped,
        prune: s.prune.read().await.clone(),
        registry_path: s.registry_path.read().await.clone(),
        registry_format: s.registry_format.read().await.clone(),
        counts,
    })
}

async fn api_reload(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    match load_registry(&s, false).await {
        Ok(()) => (StatusCode::OK, axum::Json(serde_json::json!({"ok": true}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": e})),
        ),
    }
}

async fn api_servers(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    s.sup.refresh_health().await;
    axum::Json(s.sup.status_all().await)
}

#[derive(Deserialize)]
struct PathBody {
    path: String,
}

async fn api_open(
    State(s): State<Arc<AppState>>,
    axum::Json(body): axum::Json<PathBody>,
) -> impl IntoResponse {
    let found = s.repos.read().await.iter().find(|r| r.path == body.path).cloned();
    let Some(target) = found else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": format!("{} is not in the shown repo list", body.path)
            })),
        );
    };
    let st = s.sup.ensure(&target).await;
    (StatusCode::OK, axum::Json(serde_json::to_value(st).unwrap_or_default()))
}

async fn api_stop(
    State(s): State<Arc<AppState>>,
    axum::Json(body): axum::Json<PathBody>,
) -> impl IntoResponse {
    let stopped = s.sup.stop(&body.path).await;
    axum::Json(serde_json::json!({ "stopped": stopped }))
}

async fn api_health(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "ok": true,
        "uptime_ms": now_ms().saturating_sub(s.started_ms),
        "web_dir": s.web_dir.display().to_string(),
        "web_dir_present": s.web_dir.is_dir(),
        "cache_dir": s.cache_dir.display().to_string(),
        "feed": "git-lex-serve sparql",
    }))
}

// ------------------------------------------------------------------- layout

/// Resolve a genesis sha to a live, verified server AND the repo it belongs
/// to, since the layout needs the repo's HEAD to key its cache.
async fn layout_target(
    s: &Arc<AppState>,
    genesis: &str,
) -> Result<(repo::RepoProbe, u16), (StatusCode, String)> {
    let path = s.by_genesis.read().await.get(genesis).cloned().ok_or((
        StatusCode::NOT_FOUND,
        format!("no repo on this machine has genesis {genesis}"),
    ))?;
    // Re-probe rather than trusting the startup snapshot.
    //
    // The layout cache is keyed on HEAD, which is only an invalidation if the
    // HEAD is current. Read once at startup it is a photograph: a repo that
    // gains commits while this is running keeps serving the graph it had when
    // the front door opened, labelled "from cache" and looking correct. Caught
    // by committing four documents to my own soul and watching the view not
    // change — the exact defect the cache's own comment warns about, in the
    // code that carries the comment.
    //
    // A probe is two cheap local git calls, and it happens once per layout
    // request, not per frame.
    let known = s.repos.read().await.iter().find(|r| r.path == path).cloned();
    let last_used = known.as_ref().and_then(|r| {
        if r.recency_source == repo::RecencySource::LastUsed { r.recency.clone() } else { None }
    });
    let probe = repo::probe(std::path::Path::new(&path), last_used).await;
    // Keep the shown list in step, so the picker's commit count and last
    // activity do not drift away from what the stage is drawing.
    {
        let mut repos = s.repos.write().await;
        if let Some(slot) = repos.iter_mut().find(|r| r.path == path) {
            *slot = probe.clone();
        }
    }
    let port = s.sup.proxy_port(&path).await.ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "no verified server is running for this repo".to_string(),
        )
    })?;
    Ok((probe, port))
}

/// Build (or reuse) the layout and return its metadata. The binary payload
/// comes from the companion `/data` route.
async fn api_layout_meta(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
) -> Response {
    let (probe, port) = match layout_target(&s, &genesis).await {
        Ok(v) => v,
        Err((c, m)) => return (c, m).into_response(),
    };
    let head = probe.head_sha.clone().unwrap_or_default();

    if let Some(c) = layout_api::load(&s.cache_dir, &genesis, &head) {
        s.layouts.write().await.insert(genesis.clone(), Arc::new(c.data));
        return (
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                // Say where it came from. A cached figure presented as
                // current is the whole defect class this header exists to
                // prevent.
                (axum::http::HeaderName::from_static("x-layout-source"), "cache"),
            ],
            c.meta_json,
        )
            .into_response();
    }

    let built = match layout_api::build_from_server(
        &s.http,
        port,
        &genesis,
        &head,
        std::path::Path::new(&probe.path),
    )
    .await
    {
        Ok(l) => l,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };
    let meta_json = match layout_api::store(&s.cache_dir, &genesis, &head, &built) {
        Ok(j) => j,
        // A cache we could not write is a slow path, not a failure.
        Err(_) => serde_json::to_string(&built.meta).unwrap_or_default(),
    };
    s.layouts.write().await.insert(genesis.clone(), Arc::new(built.data));
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::HeaderName::from_static("x-layout-source"), "computed"),
        ],
        meta_json,
    )
        .into_response()
}

/// The packed typed arrays: positions, colours, sizes, edge indices. Goes
/// straight into GPU buffers, so it travels as bytes and never as JSON.
async fn api_layout_data(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
) -> Response {
    if let Some(d) = s.layouts.read().await.get(&genesis).cloned() {
        return (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            (*d).clone(),
        )
            .into_response();
    }
    (
        StatusCode::NOT_FOUND,
        "no layout has been built for this repo yet — request its metadata first",
    )
        .into_response()
}

// -------------------------------------------------------------------- proxy

/// Resolve a genesis sha to the port of a *verified, currently answering*
/// server for that repo. Anything less than that returns an error the page
/// can show, never a silent fallback to whatever is listening.
async fn resolve_port(s: &Arc<AppState>, genesis: &str) -> Result<u16, (StatusCode, String)> {
    let path = s
        .by_genesis
        .read()
        .await
        .get(genesis)
        .cloned()
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("no repo on this machine has genesis {genesis}"),
        ))?;
    match s.sup.proxy_port(&path).await {
        Some(p) => Ok(p),
        None => {
            let st = s.sup.status_of(&path).await;
            let detail = st
                .as_ref()
                .and_then(|x| x.message.clone())
                .unwrap_or_else(|| "no server is running for this repo".to_string());
            Err((StatusCode::SERVICE_UNAVAILABLE, detail))
        }
    }
}

/// Pass a SPARQL query through to one repo's endpoint.
///
/// The browser never learns a port. It addresses a soul by its genesis sha —
/// its permanent identity — and this resolves that to a server that is
/// currently answering AND has been confirmed to be that repo. A port is a
/// place; it is not a name, and on a machine running a dozen souls the
/// difference is one soul's page showing another soul's data.
async fn proxy_sparql(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let port = match resolve_port(&s, &genesis).await {
        Ok(p) => p,
        Err((c, m)) => return (c, m).into_response(),
    };
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/sparql-query")
        .to_string();
    let url = format!("http://127.0.0.1:{port}/sparql");
    match s
        .http
        .post(&url)
        .header("content-type", ct)
        .header("accept", "application/sparql-results+json")
        .body(body)
        .send()
        .await
    {
        Ok(r) => relay(r).await,
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("the data endpoint for this repo stopped answering: {e}"),
        )
            .into_response(),
    }
}

async fn relay(r: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(r.status().as_u16()).unwrap_or(StatusCode::OK);
    let ct = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    match r.bytes().await {
        Ok(b) => (status, [(axum::http::header::CONTENT_TYPE, ct)], b).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("the reply from the repo endpoint was cut off: {e}"),
        )
            .into_response(),
    }
}

// ------------------------------------------------------------------- static

async fn index(State(s): State<Arc<AppState>>) -> Response {
    match std::fs::read_to_string(s.web_dir.join("index.html")) {
        Ok(html) => Html(html).into_response(),
        Err(_) => Html(format!(
            "<pre style=\"font:14px ui-monospace,monospace;padding:2rem\">\
             git-lex-ui is running, but the frontend is not built.\n\n\
             Looked in: {}\n\n\
             Build it with:  cd web &amp;&amp; npm install &amp;&amp; npm run build\n\
             Or point elsewhere with:  git-lex-ui --web &lt;dir&gt;\n</pre>",
            s.web_dir.display()
        ))
        .into_response(),
    }
}

async fn static_asset(State(s): State<Arc<AppState>>, AxPath(asset): AxPath<String>) -> Response {
    // Serve from disk on every request so the frontend can be edited live —
    // the same property that makes the per-repo servers editable.
    let rel = PathBuf::from(asset.trim_start_matches('/'));
    let full = s.web_dir.join(&rel);
    let root = s.web_dir.canonicalize().unwrap_or_else(|_| s.web_dir.clone());
    let canon = match full.canonicalize() {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };
    if !canon.starts_with(&root) {
        return (StatusCode::FORBIDDEN, "outside the web root").into_response();
    }
    let ct = match canon.extension().and_then(|e| e.to_str()) {
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    };
    match std::fs::read(&canon) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, ct)], bytes).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// The text of one document, read straight off disk.
///
/// Rob asked for the old viewer's behaviour: click a dot, see the actual
/// file. That is a filesystem read, not a graph query — the store holds the
/// document's *facts*, never its prose, so no amount of SPARQL returns the
/// words. This front door already knows every repo's absolute path, so it can
/// answer directly.
///
/// `path` is the repo-relative path carried in the File IRI
/// (`https://repolex.ai/git-lex/File/Soul/Note/x.md` -> `Soul/Note/x.md`),
/// which is why the client can ask for it without a second lookup.
///
/// **It is attacker-supplied, so it is checked rather than trusted.** The
/// resolved path must stay inside the repo after canonicalisation, which
/// closes `../` traversal and a symlink pointing out of the tree — the escape
/// that a plain string check on `..` misses. This binds to localhost, but a
/// local read-anything endpoint is worth exactly one function's care.
#[derive(serde::Deserialize)]
struct FileQuery {
    path: String,
}

async fn api_file(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> Response {
    let probe = {
        let repos = s.repos.read().await;
        repos
            .iter()
            .find(|r| r.genesis_sha.as_deref() == Some(genesis.as_str()))
            .cloned()
    };
    let Some(probe) = probe else {
        return (StatusCode::NOT_FOUND, "no such repo").into_response();
    };

    let root = std::path::Path::new(&probe.path);
    let Ok(root) = root.canonicalize() else {
        return (StatusCode::NOT_FOUND, "repo path is gone").into_response();
    };
    let Ok(full) = root.join(&q.path).canonicalize() else {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    };
    if !full.starts_with(&root) {
        return (StatusCode::FORBIDDEN, "outside the repo").into_response();
    }

    match tokio::fs::read_to_string(&full).await {
        Ok(text) => {
            let bytes = text.len();
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::json!({
                    "path": q.path,
                    "bytes": bytes,
                    "text": text,
                })
                .to_string(),
            )
                .into_response()
        }
        // A file in the graph that is not on disk is not an error: the graph
        // is a record of history and the document may have been deleted since.
        // Say which, rather than 500-ing on a true fact about the past.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "not on disk — deleted since it was recorded").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
