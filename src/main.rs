//! git-lex-ui — a front door to every git-lex repo on the machine.
//!
//! It starts once on a fixed port, reads the machine registry, drops the
//! entries that have rotted, and draws whichever repo you pick.
//!
//! The data comes from `gitlexd`, the one store daemon on the machine, which
//! holds every registered repo and answers for each one by its genesis sha.
//! Until 2026-09-22 this file supervised a `git-lex-serve sparql` process per
//! repo and translated genesis shas into port numbers, because a port number
//! is a place and not a name. git-lex removed that command and its binary
//! (merge f428987, @w4r3z) in favour of the daemon, which takes the name
//! directly — so the translation, and the twenty-eight processes it used to
//! keep alive, are both gone. See `daemon.rs`.
//!
//! The browser still only ever talks to this process.

mod daemon;
mod layout;
mod layout_api;
mod registry;
mod repo;
mod sparql;

use axum::{
    Router,
    extract::{Path as AxPath, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use daemon::Daemon;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
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

    /// Where `gitlexd` listens. Fixed by git-lex; a flag only so a test
    /// daemon can be pointed at.
    #[arg(long, default_value_t = daemon::DAEMON_PORT)]
    daemon_port: u16,

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
    lexd: Arc<Daemon>,
    /// What the daemon looked like at the last poll. Refreshed on a timer, so
    /// a page left open for an hour finds out when the feed dies.
    lexd_status: RwLock<daemon::DaemonStatus>,
    http: reqwest::Client,
    web_dir: PathBuf,
    lex_dir: PathBuf,
    cache_dir: PathBuf,
    /// Packed layout payloads, by genesis sha, for the companion data route.
    layouts: RwLock<HashMap<String, Arc<Vec<u8>>>>,
    /// What `git lex sync` is doing, per repo path. See `SyncState`.
    syncs: RwLock<HashMap<String, SyncState>>,
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

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_default();
    let lexd = Arc::new(Daemon::new(http.clone(), args.daemon_port));

    // Ask before drawing anything. The daemon is usually already up — `git
    // lex query` starts one on demand — and when it is not, starting it is
    // safe to do blind, because the port is the lock and a second daemon
    // exits on its own.
    let boot = lexd.ensure().await;
    if boot.reachable {
        println!(
            "gitlexd on port {}: {} soul(s) held{}",
            boot.port,
            boot.souls.len(),
            boot.message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default()
        );
    } else {
        eprintln!(
            "git-lex-ui: {}",
            boot.message.as_deref().unwrap_or("gitlexd is not answering")
        );
        eprintln!("The front door still starts — you just cannot draw anything until it is.");
    }

    let state = Arc::new(AppState {
        repos: RwLock::new(vec![]),
        by_genesis: RwLock::new(HashMap::new()),
        prune: RwLock::new(None),
        dropped: RwLock::new(vec![]),
        lexd: Arc::clone(&lexd),
        lexd_status: RwLock::new(boot),
        http,
        web_dir: web_dir.clone(),
        lex_dir: lex_dir.clone(),
        cache_dir: cache_dir.clone(),
        layouts: RwLock::new(HashMap::new()),
        syncs: RwLock::new(HashMap::new()),
        registry_path: RwLock::new(None),
        registry_format: RwLock::new(None),
        started_ms: now_ms(),
    });

    if let Err(e) = load_registry(&state, !args.no_prune).await {
        eprintln!("git-lex-ui: {e}");
        eprintln!("The front door still starts — you just get an empty list until this is fixed.");
    }

    let app = Router::new()
        .route("/", get(index))
        .route("/api/repos", get(api_repos))
        .route("/api/repos/reload", post(api_reload))
        .route("/api/feed", get(api_feed))
        .route("/api/health", get(api_health))
        .route("/api/layout/{genesis}", get(api_layout_meta))
        .route("/api/layout/{genesis}/data", get(api_layout_data))
        .route("/api/sync", get(api_sync_status).post(api_sync_start))
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
    println!("Data feed: gitlexd on 127.0.0.1:{}", args.daemon_port);
    if !args.no_open {
        let _ = open::that_detached(&url);
    }

    // Health is asked on a timer, not remembered. A page left open for an
    // hour finds out its feed died.
    let health_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            let next = health_state.lexd.status().await;
            *health_state.lexd_status.write().await = next;
        }
    });

    // Nothing of ours to shut down any more. The daemon belongs to the
    // machine, not to this process — leaving it running is correct, and
    // killing it would take the store out from under `git lex query` in every
    // other terminal on the box.
    let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!();
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

    dedupe_by_genesis(&mut probes, |p| {
        std::path::Path::new(p)
            .canonicalize()
            .map(|c| c == std::path::Path::new(p))
            .unwrap_or(false)
    });

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

/// One row per repo, not one row per path that reaches it.
///
/// The registry is a list of absolute paths, and on 2026-09-22 every repo
/// moved from `~/repos` to `/Volumes/f00/repos` with `~/repos` left behind as
/// a symlink. Nine repos ended up in the file under both spellings, and
/// because each row is probed independently, both spellings passed every
/// liveness check — so the picker showed nine souls twice. Neither row was
/// wrong. They were the same repo.
///
/// A path is a place and the genesis sha is the name, so the dedupe is on the
/// name. The survivor is the row whose path is already canonical — the real
/// location rather than a route to it — because that is the path handed to
/// `git -C` and shown in the interface. When neither is canonical, or both
/// are, the first one wins, which keeps the registry's own order.
fn dedupe_by_genesis(probes: &mut Vec<repo::RepoProbe>, canonical: impl Fn(&str) -> bool) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut keep = vec![true; probes.len()];
    for i in 0..probes.len() {
        // A repo with no genesis sha has no name to be deduped on, and there
        // is nothing to gain by guessing one. Every such row is kept.
        let Some(g) = probes[i].genesis_sha.clone() else { continue };
        match seen.get(&g).copied() {
            None => {
                seen.insert(g, i);
            }
            Some(first) => {
                let loser = if canonical(&probes[i].path) && !canonical(&probes[first].path) {
                    seen.insert(g, i);
                    first
                } else {
                    i
                };
                keep[loser] = false;
            }
        }
    }
    let mut it = keep.iter();
    probes.retain(|_| *it.next().unwrap_or(&true));
}

/// Replace each row's spine-derived freshness with the daemon's own answer.
///
/// `gitlexd` holds every store open and syncs it itself, so it knows which
/// commit each graph was built up to. The spine file on disk is a marker for
/// the same fact — still truthfully written, but read here through an
/// inference that broke: "a store file is newer than the spine" was taken to
/// mean "the graph moved past its marker", and opening a store rewrites its
/// files without moving anything. The daemon opens every store when it
/// starts, so on 2026-09-22 that put a warning triangle beside six repos
/// synced to exactly HEAD. Ask the writer instead of reading the traces.
///
/// The spine reading is kept for any repo the daemon does not hold, which is
/// the only case where nobody current can be asked.
async fn overlay_daemon_freshness(s: &Arc<AppState>) {
    let st = s.lexd.status().await;
    *s.lexd_status.write().await = st.clone();
    if !st.reachable {
        return;
    }
    let by_genesis: HashMap<&str, &daemon::Soul> =
        st.souls.iter().map(|x| (x.genesis.as_str(), x)).collect();

    let mut repos = s.repos.write().await;
    for r in repos.iter_mut() {
        let Some(g) = r.genesis_sha.as_deref() else { continue };
        let Some(soul) = by_genesis.get(g) else { continue };
        r.graph = repo::GraphFreshness::from_daemon(
            soul.synced_to.as_deref(),
            r.head_sha.as_deref(),
            soul.syncing,
        );
    }
    drop(repos);

    // Count the distance for anything behind. One `git rev-list` per behind
    // repo, and normally there are none — the daemon syncs within a couple of
    // seconds of a commit — so this is the rare path, not the common one.
    let behind: Vec<(String, String, String)> = s
        .repos
        .read()
        .await
        .iter()
        .filter_map(|r| match (&r.graph, &r.head_sha) {
            (repo::GraphFreshness::Behind { sha, commits: None }, Some(head)) => {
                Some((r.path.clone(), sha.clone(), head.clone()))
            }
            _ => None,
        })
        .collect();
    if behind.is_empty() {
        return;
    }
    let counted = futures_util::future::join_all(behind.into_iter().map(|(path, sha, head)| async move {
        let n = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["rev-list", "--count", &format!("{sha}..{head}")])
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok());
        (path, sha, n)
    }))
    .await;
    let mut repos = s.repos.write().await;
    for (path, sha, commits) in counted {
        if let Some(r) = repos.iter_mut().find(|r| r.path == path) {
            r.graph = repo::GraphFreshness::Behind { sha, commits };
        }
    }
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
    overlay_daemon_freshness(&s).await;
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

/// The state of the one data feed, and which souls it holds.
///
/// This replaced three routes — list the per-repo servers, start one, stop
/// one — because there is nothing per-repo left to start. A page that cannot
/// draw now has exactly one thing to check, and it is named here rather than
/// inferred from a column of unreachable children.
///
/// It asks the daemon rather than serving the timer's snapshot, so a page
/// that opens right after the daemon comes back does not have to wait out the
/// poll to find that out.
async fn api_feed(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let fresh = s.lexd.status().await;
    *s.lexd_status.write().await = fresh.clone();
    axum::Json(fresh)
}

async fn api_health(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "ok": true,
        "uptime_ms": now_ms().saturating_sub(s.started_ms),
        "web_dir": s.web_dir.display().to_string(),
        "web_dir_present": s.web_dir.is_dir(),
        "cache_dir": s.cache_dir.display().to_string(),
        "feed": "gitlexd",
        "feed_port": s.lexd.port(),
        "feed_reachable": s.lexd_status.read().await.reachable,
    }))
}

// ------------------------------------------------------------------- layout

/// Resolve a genesis sha to the repo it belongs to, re-probed, since the
/// layout needs the repo's HEAD to key its cache.
///
/// This used to also resolve a port, and to fail when no child server was
/// answering for that repo. The daemon holds every registered soul, so the
/// only remaining way to have nothing to ask is for the daemon itself to be
/// down — which `held_by_daemon` reports as one fact about the feed rather
/// than as this repo being broken.
async fn layout_target(
    s: &Arc<AppState>,
    genesis: &str,
) -> Result<repo::RepoProbe, (StatusCode, String)> {
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
    held_by_daemon(s, genesis).await?;
    Ok(probe)
}

/// Confirm the daemon is answering AND holds this soul, before anything asks
/// it a question.
///
/// The two failures read very differently to whoever is looking at the page,
/// so they are never merged: a daemon that is down is one problem for the
/// whole machine, and a soul the daemon does not hold is one repo that git-lex
/// has not been run in. The second used to be impossible to express — the
/// supervisor would simply start a server — and saying "nothing is answering"
/// for it would send someone to restart a daemon that is fine.
async fn held_by_daemon(s: &Arc<AppState>, genesis: &str) -> Result<(), (StatusCode, String)> {
    let st = s.lexd.status().await;
    *s.lexd_status.write().await = st.clone();
    if !st.reachable {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            st.message.unwrap_or_else(|| {
                format!("gitlexd is not answering on port {}", st.port)
            }),
        ));
    }
    if st.souls.iter().any(|x| x.genesis == genesis) {
        return Ok(());
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        format!(
            "gitlexd is running but does not hold this repo — run `git lex sync` in it once, \
             and it will be picked up"
        ),
    ))
}

#[derive(Deserialize)]
struct ViewQuery {
    #[serde(default)]
    view: Option<String>,
}

/// `?view=base` or `?view=typed`; typed when absent, which is what every
/// link made before the base view existed meant. An unknown value is an
/// error rather than a quiet default — a misspelt view drawing the other one
/// would look like a correct answer.
fn view_of(q: &ViewQuery) -> Result<layout::View, Response> {
    match q.view.as_deref() {
        None => Ok(layout::View::Typed),
        Some(v) => layout::View::parse(v).ok_or_else(|| {
            (StatusCode::BAD_REQUEST, format!("unknown view `{v}` — use base or typed")).into_response()
        }),
    }
}

/// In-memory payloads are held per repo AND per view.
fn layout_key(genesis: &str, view: layout::View) -> String {
    format!("{genesis}:{}", view.as_str())
}

/// Build (or reuse) the layout and return its metadata. The binary payload
/// comes from the companion `/data` route.
async fn api_layout_meta(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
    axum::extract::Query(q): axum::extract::Query<ViewQuery>,
) -> Response {
    let view = match view_of(&q) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let probe = match layout_target(&s, &genesis).await {
        Ok(v) => v,
        Err((c, m)) => return (c, m).into_response(),
    };
    let head = probe.head_sha.clone().unwrap_or_default();
    let key = layout_key(&genesis, view);

    if let Some(c) = layout_api::load(&s.cache_dir, &genesis, &head, view) {
        s.layouts.write().await.insert(key, Arc::new(c.data));
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
        s.lexd.port(),
        &genesis,
        &head,
        std::path::Path::new(&probe.path),
        view,
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
    s.layouts.write().await.insert(key, Arc::new(built.data));
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
    axum::extract::Query(q): axum::extract::Query<ViewQuery>,
) -> Response {
    let view = match view_of(&q) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Some(d) = s.layouts.read().await.get(&layout_key(&genesis, view)).cloned() {
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

/// Pass a SPARQL query through to one soul on the daemon.
///
/// The browser never learns a port — it addresses a soul by its genesis sha,
/// and so, now, does the daemon. This used to be the place where a name was
/// turned into a port, and the care it took was the difference between one
/// soul's page and another soul's data. That care is now structural: there is
/// one port for the machine and the name travels all the way down.
async fn proxy_sparql(
    State(s): State<Arc<AppState>>,
    AxPath(genesis): AxPath<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if s.by_genesis.read().await.get(&genesis).is_none() {
        return (
            StatusCode::NOT_FOUND,
            format!("no repo on this machine has genesis {genesis}"),
        )
            .into_response();
    }
    if let Err((c, m)) = held_by_daemon(&s, &genesis).await {
        return (c, m).into_response();
    }
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/sparql-query")
        .to_string();
    let url = format!("{}/sparql", s.lexd.soul_base(&genesis));
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
            format!("gitlexd stopped answering for this repo: {e}"),
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
/// Requested by @goodlux: the old viewer's behaviour, click a dot and see
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


// --- asking the daemon to sync one soul -----------------------------------
//
// The only write this tool performs, and it is deliberately the narrowest one
// possible: it rebuilds a DERIVED store from commits that already exist. It
// creates no commits, changes no tracked file, and loses nothing that is not
// regenerable from git.
//
// It used to shell out to `git lex sync` in the repo's directory. It asks the
// daemon now, and that is a correctness change rather than a tidy-up: gitlexd
// holds every store OPEN, so a second process writing the same database is
// two writers on one file. Asking the process that already holds it is the
// only way to have one.
//
// It earns its place because "13 of 15 graphs are behind" was true for two
// weeks and the answer was always the same command in a different directory.
// A banner that states a problem you have to go elsewhere to fix is a label
// with no action attached — it was replaced by this. The daemon now also
// syncs within a couple of seconds of any commit on its own, so this is the
// button for the case where that did not happen, not the normal path.

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum SyncState {
    Running { started_ms: u128 },
    /// Finished cleanly. `summary` is built from the daemon's own reply, so
    /// the figures shown are its, not ours.
    Done { ms: u128, summary: String },
    /// Finished badly. The message is kept verbatim: a sync failure is
    /// usually a real fact about the repo and paraphrasing it loses the fix.
    Failed { ms: u128, message: String },
}

async fn api_sync_status(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(s.syncs.read().await.clone())
}

#[derive(serde::Deserialize)]
struct SyncRequest {
    path: String,
}

async fn api_sync_start(
    State(s): State<Arc<AppState>>,
    axum::Json(body): axum::Json<SyncRequest>,
) -> Response {
    // Only repos this front door already knows about. The path arrives from
    // the browser, and a sync of an arbitrary directory is not something a
    // page should be able to ask for.
    let known = s
        .repos
        .read()
        .await
        .iter()
        .find(|r| r.path == body.path)
        .and_then(|r| r.genesis_sha.clone());
    let Some(genesis) = known else {
        return (
            StatusCode::NOT_FOUND,
            "no such repo in the registry, or it has no genesis commit",
        )
            .into_response();
    };

    {
        let syncs = s.syncs.read().await;
        if matches!(syncs.get(&body.path), Some(SyncState::Running { .. })) {
            return (StatusCode::CONFLICT, "already syncing").into_response();
        }
    }

    let started = now_ms();
    s.syncs
        .write()
        .await
        .insert(body.path.clone(), SyncState::Running { started_ms: started });

    let st = s.clone();
    let path = body.path.clone();
    tokio::spawn(async move {
        let out = st.lexd.sync(&genesis).await;
        let ms = now_ms().saturating_sub(started);
        let next = match out {
            Ok(soul) => {
                // Say what actually happened, from the daemon's own numbers.
                // A sync that failed inside the daemon still returns 200 with
                // `last_error` set, so a reply that parsed is not by itself a
                // success — the field has to be read.
                if let Some(err) = soul.last_error.filter(|e| !e.trim().is_empty()) {
                    SyncState::Failed { ms, message: err }
                } else {
                    let took = soul
                        .last_sync_ms
                        .map(|n| format!("Synced in {n}ms"))
                        .unwrap_or_else(|| "Synced".to_string());
                    let at = soul
                        .synced_to
                        .as_deref()
                        .map(|sha| format!(" — store now at {}", &sha[..sha.len().min(8)]))
                        .unwrap_or_default();
                    SyncState::Done { ms, summary: format!("{took}{at}") }
                }
            }
            Err(message) => SyncState::Failed { ms, message },
        };
        st.syncs.write().await.insert(path.clone(), next);

        // Drop this repo's cached layouts. They were built from the old
        // store, and a cache keyed by HEAD cannot see that the store beneath
        // it changed — the same blindness that made LAYOUT_VERSION necessary.
        let dir = st.cache_dir.join(&genesis);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("layout-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let mut held = st.layouts.write().await;
        held.remove(&layout_key(&genesis, layout::View::Base));
        held.remove(&layout_key(&genesis, layout::View::Typed));
    });

    (StatusCode::ACCEPTED, axum::Json(SyncState::Running { started_ms: started })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two fields the dedupe reads, and nothing else. Everything a probe
    /// carries is about DISPLAYING a repo; the dedupe is about identifying
    /// one, and the test should fail if that ever stops being true.
    fn probe(path: &str, genesis: Option<&str>) -> repo::RepoProbe {
        repo::RepoProbe {
            path: path.to_string(),
            genesis_sha: genesis.map(str::to_string),
            name: String::new(),
            name_declared: false,
            agent_name: None,
            kit: None,
            family: repo::RepoFamily::Plain,
            optional_kits: vec![],
            head_sha: None,
            head_time: None,
            commit_count: None,
            recency: None,
            recency_source: repo::RecencySource::Unknown,
            graph: repo::GraphFreshness::NeverSynced,
            has_www: false,
            warnings: vec![],
        }
    }

    /// The case that put nine souls in the list twice: one repo reachable at
    /// two spellings after the volume move, both of them live.
    #[test]
    fn two_paths_to_one_repo_collapse_to_the_real_one() {
        let mut v = vec![
            probe("/Users/rob/repos/W3BL0RD", Some("e3d71e")),
            probe("/Volumes/f00/repos/W3BL0RD", Some("e3d71e")),
        ];
        dedupe_by_genesis(&mut v, |p| p.starts_with("/Volumes"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, "/Volumes/f00/repos/W3BL0RD");
    }

    /// Order in the file must not decide the answer. The same pair the other
    /// way round keeps the same survivor.
    #[test]
    fn the_real_path_wins_whichever_way_round_the_rows_are() {
        let mut v = vec![
            probe("/Volumes/f00/repos/lUX", Some("aaa")),
            probe("/Users/rob/repos/lUX", Some("aaa")),
        ];
        dedupe_by_genesis(&mut v, |p| p.starts_with("/Volumes"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, "/Volumes/f00/repos/lUX");
    }

    /// Different repos are never merged, however alike their paths look. The
    /// dedupe is on the name, and two names are two repos.
    #[test]
    fn repos_with_different_genesis_shas_are_left_alone() {
        let mut v = vec![
            probe("/Volumes/f00/repos/a", Some("aaa")),
            probe("/Volumes/f00/repos/b", Some("bbb")),
        ];
        dedupe_by_genesis(&mut v, |_| true);
        assert_eq!(v.len(), 2);
    }

    /// A repo with no genesis sha has no name, so it cannot be deduped and
    /// must not be dropped. Several of them do not collapse into one.
    #[test]
    fn rows_without_a_genesis_sha_are_all_kept() {
        let mut v = vec![
            probe("/Volumes/f00/repos/x", None),
            probe("/Volumes/f00/repos/y", None),
            probe("/Volumes/f00/repos/z", Some("ccc")),
        ];
        dedupe_by_genesis(&mut v, |_| true);
        assert_eq!(v.len(), 3);
    }
}
