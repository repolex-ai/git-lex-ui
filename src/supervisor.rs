//! Starting and watching the per-repo `git lex serve viz` processes.
//!
//! Three things about the child process shape this whole module, and all
//! three were read out of `git-lex/src/bin/git-lex-serve.rs` rather than
//! assumed.
//!
//! **1. It takes no `--repo`.** The only flag is `--port`. The child resolves
//! which repository it is serving by calling `find_git_root()` on its own
//! working directory. So the repo is selected by `current_dir` on the spawn,
//! and getting that wrong does not error — it silently serves a different
//! repo, or the one this binary is running from.
//!
//! **2. It picks its own port.** Given `--port N` it walks `N..N+20` and
//! binds the first that is free, printing `Port N was taken, using M
//! instead`. So the port we asked for is *not* the port we got, and in a
//! machine with sixteen souls and a live registry the race window is real.
//! We therefore never assume: we read the port back off the child's stdout,
//! and then we verify the identity of what answered.
//!
//! **3. It is reached directly, never through `git lex serve`.** That
//! wrapper is three processes deep — `git` to `git-lex` to `git-lex-serve` —
//! and only the innermost one binds the port. See the spawn site.
//!
//! **4. It opens a browser, and it cannot be told not to.** The child calls
//! `open::that_detached(url)` on startup with no flag to disable it.
//! Harmless when a human runs one by hand; when a supervisor starts one per
//! soul, every click in this UI also opens a tab showing the OLD viewer,
//! which looks exactly like the new interface linking back to the old one.
//! See `BrowserGuard`.
//!
//! The identity check is the load-bearing part. A supervisor that remembers
//! it started a server on port 7901 and later finds *something* answering on
//! 7901 has learned nothing — on 2026-08-27 two dev servers exited between
//! one message and the next while the page kept rendering the last data it
//! had and looked entirely fine. Health is asked, never remembered, and
//! "asked" means asking *who are you*, not *are you up*.

use crate::repo::RepoProbe;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

/// The `repo` named graph the child serves, which carries `gl:genesisSha`.
const REPO_GRAPH: &str = "https://repolex.ai/git-lex/NamedGraph/repo";
const GL_GENESIS: &str = "https://repolex.ai/ontology/git-lex/genesisSha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerState {
    /// Spawned; has not yet announced a port.
    Starting,
    /// Answering, and it answered with the genesis sha we expected.
    Ready,
    /// Answering, but it is not the repo we meant to start. Never proxied.
    IdentityMismatch,
    /// Was ready, is not answering now.
    Unreachable,
    /// The process exited.
    Exited,
    /// Never got off the ground.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub path: String,
    pub genesis_sha: Option<String>,
    pub state: ServerState,
    /// The port the child actually bound, read from its own output. `None`
    /// until it says.
    pub port: Option<u16>,
    /// The port we asked for, kept so a walk is visible rather than silent.
    pub requested_port: u16,
    pub pid: Option<u32>,
    /// Where the child is serving its frontend from. A repo can be serving a
    /// stale copy of the UI, and that should be visible, not a mystery.
    pub www_dir: Option<String>,
    /// Millis since the epoch of the last successful identity-checked probe.
    pub last_seen_ms: Option<u128>,
    /// True when this process was already running at startup and was taken
    /// over rather than started. An adopted server has no stdout pipe, so its
    /// death is noticed by the health poll rather than instantly.
    pub adopted: bool,
    pub message: Option<String>,
}

pub struct Supervisor {
    servers: RwLock<HashMap<String, ServerStatus>>,
    children: RwLock<HashMap<String, Child>>,
    http: reqwest::Client,
    /// How children are stopped from opening browser tabs. Chosen once at
    /// startup and reported, never assumed.
    guard: BrowserGuard,
    port_floor: u16,
    /// Where the pids of live children are recorded, so a supervisor that is
    /// killed rather than shut down can find them again next time.
    persist_path: PathBuf,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

impl Supervisor {
    pub fn new(port_floor: u16, cache_dir: &std::path::Path) -> Arc<Self> {
        Arc::new(Self {
            servers: RwLock::new(HashMap::new()),
            children: RwLock::new(HashMap::new()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            guard: choose_browser_guard(cache_dir),
            port_floor,
            persist_path: cache_dir.join("children.json"),
        })
    }

    pub fn guard(&self) -> &BrowserGuard {
        &self.guard
    }

    pub async fn status_all(&self) -> Vec<ServerStatus> {
        let mut v: Vec<_> = self.servers.read().await.values().cloned().collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    pub async fn status_of(&self, path: &str) -> Option<ServerStatus> {
        self.servers.read().await.get(path).cloned()
    }

    /// The port to proxy to, but only for a server that is Ready. An
    /// unreachable or mismatched server yields nothing, so a stale page gets
    /// a real error instead of somebody else's data.
    pub async fn proxy_port(&self, path: &str) -> Option<u16> {
        let s = self.servers.read().await;
        let st = s.get(path)?;
        if st.state == ServerState::Ready { st.port } else { None }
    }

    /// Start a server for a repo, or return the existing one if it is
    /// genuinely still answering as the right repo.
    pub async fn ensure(self: &Arc<Self>, repo: &RepoProbe) -> ServerStatus {
        if let Some(existing) = self.servers.read().await.get(&repo.path).cloned() {
            if existing.state == ServerState::Ready {
                // Do not trust the record. Ask.
                if let Some(port) = existing.port {
                    if self.verify(port, repo.genesis_sha.as_deref()).await.is_ok() {
                        return existing;
                    }
                }
            }
        }
        self.spawn(repo).await
    }

    async fn spawn(self: &Arc<Self>, repo: &RepoProbe) -> ServerStatus {
        let st = self.spawn_once(repo, true).await;
        // `sandbox-exec` is deprecated on macOS. If it ever stops working,
        // a browser tab is a far smaller problem than a front door that
        // cannot open anything — so fall back, once, and say so.
        if st.state == ServerState::Failed && matches!(self.guard, BrowserGuard::Sandbox(_)) {
            eprintln!(
                "sandbox-exec could not start the server for {}; retrying unguarded — expect a browser tab",
                repo.path
            );
            let mut retried = self.spawn_once(repo, false).await;
            if retried.state == ServerState::Ready {
                retried.message = Some(
                    "started without the browser guard — sandbox-exec failed, so this one opened a tab"
                        .to_string(),
                );
                self.servers.write().await.insert(repo.path.clone(), retried.clone());
            }
            return retried;
        }
        st
    }

    async fn spawn_once(self: &Arc<Self>, repo: &RepoProbe, guarded: bool) -> ServerStatus {
        let requested = self.pick_port().await;
        let mut status = ServerStatus {
            path: repo.path.clone(),
            genesis_sha: repo.genesis_sha.clone(),
            state: ServerState::Starting,
            port: None,
            requested_port: requested,
            pid: None,
            www_dir: None,
            last_seen_ms: None,
            adopted: false,
            message: None,
        };
        self.servers
            .write()
            .await
            .insert(repo.path.clone(), status.clone());

        // Spawn `git-lex-serve` directly, NOT `git lex serve viz`.
        //
        // Measured 2026-09-01: `git lex serve viz` is a three-deep chain —
        // `git` execs `git-lex`, which spawns `git-lex-serve`, which is the
        // process that actually binds the port. Killing what we spawned
        // killed the outermost wrapper and left the real server listening,
        // still answering, with our supervisor reporting it as `ready`
        // because it genuinely was. A stop that does not stop, and a death
        // detector watching a pipe the surviving grandchild still holds
        // open — the exact shape of the failure this module exists to
        // prevent, reproduced inside the module preventing it.
        //
        // `git-lex-serve` ships as its own binary on PATH beside `git-lex`,
        // so there is no wrapper to own. One process, one pid, one stdout.
        let mut cmd = match (&self.guard, guarded) {
            // Deny the one exec, keep everything else. The crate does not
            // check whether its browser launch succeeded, so the server
            // starts normally.
            (BrowserGuard::Sandbox(profile), true) => {
                let mut c = Command::new("sandbox-exec");
                c.arg("-f").arg(profile).arg("git-lex-serve");
                c
            }
            _ => Command::new("git-lex-serve"),
        };
        cmd.arg("viz")
            .arg("--port")
            .arg(requested.to_string())
            // The child identifies its repo by its own cwd. This line is the
            // entire repo selection mechanism.
            .current_dir(&repo.path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if guarded && matches!(self.guard, BrowserGuard::Flag) {
            cmd.arg("--no-open");
        }
        if guarded {
            if let BrowserGuard::PathShim(p) = &self.guard {
                cmd.env("PATH", p);
            }
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                status.state = ServerState::Failed;
                status.message = Some(format!("could not start git-lex-serve: {e}"));
                self.servers
                    .write()
                    .await
                    .insert(repo.path.clone(), status.clone());
                return status;
            }
        };
        status.pid = child.id();

        let stdout = child.stdout.take();
        self.children
            .write()
            .await
            .insert(repo.path.clone(), child);
        self.servers
            .write()
            .await
            .insert(repo.path.clone(), status.clone());

        // Read the port back out of the child rather than assuming the one we
        // asked for. This is the whole defence against the port walk.
        let announced = match stdout {
            Some(out) => read_announcement(out, repo.path.clone(), Arc::clone(self)).await,
            None => None,
        };

        let mut status = self
            .servers
            .read()
            .await
            .get(&repo.path)
            .cloned()
            .unwrap_or(status);

        let Some((port, www)) = announced else {
            status.state = ServerState::Failed;
            status.message =
                Some("the server started but never announced a port".to_string());
            self.servers
                .write()
                .await
                .insert(repo.path.clone(), status.clone());
            return status;
        };

        status.port = Some(port);
        status.www_dir = www;
        if port != requested {
            status.message = Some(format!(
                "asked for port {requested}, the server took {port} — {requested} was in use"
            ));
        }

        match self.verify(port, repo.genesis_sha.as_deref()).await {
            Ok(()) => {
                status.state = ServerState::Ready;
                status.last_seen_ms = Some(now_ms());
            }
            Err(why) => {
                status.state = ServerState::IdentityMismatch;
                status.message = Some(why);
            }
        }
        self.servers
            .write()
            .await
            .insert(repo.path.clone(), status.clone());
        self.persist().await;
        status
    }

    /// Record the pids of everything currently running.
    ///
    /// `kill_on_drop` only fires when this process unwinds. Killed outright —
    /// which is what a `pkill` during development does, and what a crash does
    /// in production — the children are reparented to init and keep serving,
    /// holding their ports, invisible to the next run. Measured during this
    /// build: three orphaned `git-lex-serve` processes on 7900, 7920 and 7940
    /// from earlier runs, each still answering. The pid file is what makes
    /// them findable again.
    async fn persist(&self) {
        let live: Vec<serde_json::Value> = self
            .servers
            .read()
            .await
            .values()
            .filter(|s| s.state == ServerState::Ready && s.pid.is_some())
            .map(|s| {
                serde_json::json!({
                    "path": s.path,
                    "genesis_sha": s.genesis_sha,
                    "port": s.port,
                    "pid": s.pid,
                })
            })
            .collect();
        if let Some(dir) = self.persist_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&live) {
            let _ = std::fs::write(&self.persist_path, text);
        }
    }

    /// Take over servers left running by a previous supervisor.
    ///
    /// Adoption is only ever on the strength of an identity check: the
    /// recorded pid must still be alive, the port must answer, and it must
    /// answer with the genesis sha of the repo we think it is serving. A pid
    /// can be reused and a port can be taken by something else entirely, so
    /// "the record says 7900 was lUX" is not evidence about what is on 7900
    /// now — only asking is.
    pub async fn adopt_existing(self: &Arc<Self>, repos: &[RepoProbe]) -> usize {
        let Ok(text) = std::fs::read_to_string(&self.persist_path) else {
            return 0;
        };
        let Ok(records): Result<Vec<serde_json::Value>, _> = serde_json::from_str(&text) else {
            return 0;
        };

        let mut adopted = 0;
        for r in records {
            let (Some(path), Some(port), Some(pid)) = (
                r.get("path").and_then(|v| v.as_str()),
                r.get("port").and_then(|v| v.as_u64()),
                r.get("pid").and_then(|v| v.as_u64()),
            ) else {
                continue;
            };
            let Some(repo) = repos.iter().find(|x| x.path == path) else {
                continue;
            };
            if !pid_is_alive(pid as u32) {
                continue;
            }
            if self.verify(port as u16, repo.genesis_sha.as_deref()).await.is_err() {
                continue;
            }
            self.servers.write().await.insert(
                path.to_string(),
                ServerStatus {
                    path: path.to_string(),
                    genesis_sha: repo.genesis_sha.clone(),
                    state: ServerState::Ready,
                    port: Some(port as u16),
                    requested_port: port as u16,
                    pid: Some(pid as u32),
                    www_dir: None,
                    last_seen_ms: Some(now_ms()),
                    adopted: true,
                    message: Some("already running when the front door started".to_string()),
                },
            );
            adopted += 1;
        }
        adopted
    }

    /// Ask the server on `port` which repo it is, and compare.
    ///
    /// `expected` of `None` means the repo itself has no commits and so has
    /// no identity to check against; we accept whatever answers and say so
    /// rather than pretending we verified something.
    async fn verify(&self, port: u16, expected: Option<&str>) -> Result<(), String> {
        let q = format!(
            "SELECT ?sha WHERE {{ GRAPH <{REPO_GRAPH}> {{ ?s <{GL_GENESIS}> ?sha }} }} LIMIT 1"
        );
        let url = format!("http://127.0.0.1:{port}/api/query");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "query": q }))
            .send()
            .await
            .map_err(|e| format!("no answer from port {port}: {e}"))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("port {port} answered with something unparseable: {e}"))?;
        let got = body
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("sha"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        match (expected, got) {
            (Some(want), Some(have)) if want == have => Ok(()),
            (Some(want), Some(have)) => Err(format!(
                "port {port} is serving a different repo — expected genesis {}, got {}",
                &want[..want.len().min(8)],
                &have[..have.len().min(8)]
            )),
            (Some(_), None) => {
                Err(format!("port {port} did not report a genesis sha"))
            }
            (None, _) => Ok(()),
        }
    }

    /// Re-probe every server we believe is up. Called on a timer and by the
    /// status endpoint, so a page that has been open for an hour finds out
    /// its server died rather than continuing to look fine.
    pub async fn refresh_health(&self) {
        let snapshot: Vec<ServerStatus> = self.servers.read().await.values().cloned().collect();
        for st in snapshot {
            let Some(port) = st.port else { continue };
            if matches!(st.state, ServerState::Failed | ServerState::Exited) {
                continue;
            }
            let ok = self.verify(port, st.genesis_sha.as_deref()).await;
            let mut servers = self.servers.write().await;
            if let Some(cur) = servers.get_mut(&st.path) {
                match ok {
                    Ok(()) => {
                        cur.state = ServerState::Ready;
                        cur.last_seen_ms = Some(now_ms());
                        cur.message = None;
                    }
                    Err(why) => {
                        cur.state = if why.contains("different repo") {
                            ServerState::IdentityMismatch
                        } else {
                            ServerState::Unreachable
                        };
                        cur.message = Some(why);
                    }
                }
            }
        }
    }

    pub async fn stop(&self, path: &str) -> bool {
        // Kill by pid, not only through the Child handle: an adopted server
        // has no handle here, and a handle-only stop silently no-ops on
        // exactly the processes most in need of stopping.
        let pid = self.servers.read().await.get(path).and_then(|s| s.pid);
        let child = self.children.write().await.remove(path);
        let mut stopped = false;
        if let Some(mut c) = child {
            stopped = c.kill().await.is_ok();
        }
        if let Some(pid) = pid {
            if pid_is_alive(pid) {
                stopped |= std::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
            }
        }
        if let Some(st) = self.servers.write().await.get_mut(path) {
            st.state = ServerState::Exited;
            st.port = None;
            st.message = Some("stopped".to_string());
        }
        self.persist().await;
        stopped
    }

    pub async fn stop_all(&self) {
        let paths: Vec<String> = self.children.read().await.keys().cloned().collect();
        for p in paths {
            self.stop(&p).await;
        }
    }

    /// Find a port that is free *right now*, starting from the floor and
    /// skipping the +20 window each child may walk into, so two children
    /// started back to back cannot land on each other's range.
    async fn pick_port(&self) -> u16 {
        let taken: Vec<u16> = self
            .servers
            .read()
            .await
            .values()
            .filter_map(|s| s.port.or(Some(s.requested_port)))
            .collect();
        let mut candidate = self.port_floor;
        while candidate < u16::MAX - 20 {
            let clashes = taken.iter().any(|t| candidate.abs_diff(*t) < 20);
            if !clashes && tokio::net::TcpListener::bind(("127.0.0.1", candidate)).await.is_ok() {
                return candidate;
            }
            candidate = candidate.saturating_add(20);
        }
        self.port_floor
    }
}

/// Consume the child's stdout, returning the port and www directory it
/// announces, then keep draining in the background so a chatty child never
/// blocks on a full pipe.
async fn read_announcement(
    stdout: tokio::process::ChildStdout,
    path: String,
    sup: Arc<Supervisor>,
) -> Option<(u16, Option<String>)> {
    let mut lines = BufReader::new(stdout).lines();
    let mut port: Option<u16> = None;
    let mut www: Option<String> = None;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let next = tokio::time::timeout_at(deadline, lines.next_line()).await;
        let line = match next {
            Err(_) => break,             // timed out waiting for it to speak
            Ok(Ok(Some(l))) => l,
            Ok(Ok(None)) => break,       // stdout closed: the child exited
            Ok(Err(_)) => break,
        };
        if let Some(p) = parse_listening_port(&line) {
            port = Some(p);
        }
        if let Some(rest) = line.strip_prefix("Serving assets from ") {
            www = Some(rest.trim().to_string());
        }
        if port.is_some() && www.is_some() {
            break;
        }
    }

    // Keep reading in the background, and notice when the pipe closes —
    // that is the process exiting, which is the event a page needs to see.
    tokio::spawn(async move {
        while let Ok(Some(_)) = lines.next_line().await {}
        if let Some(st) = sup.servers.write().await.get_mut(&path) {
            if st.state != ServerState::Exited {
                st.state = ServerState::Exited;
                st.port = None;
                st.message = Some("the server process exited".to_string());
            }
        }
    });

    port.map(|p| (p, www))
}

/// Is this pid still a running process? `kill -0` asks without signalling.
fn pid_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `git-lex-serve viz listening on http://127.0.0.1:7901`
fn parse_listening_port(line: &str) -> Option<u16> {
    let idx = line.find("127.0.0.1:")?;
    let tail = &line[idx + "127.0.0.1:".len()..];
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// How a child is stopped from opening a browser tab.
///
/// `git-lex-serve` calls `open::that_detached(url)` unconditionally at
/// startup and offers no flag to suppress it. That is fine for a human
/// starting one server; here it means every soul opened in this UI also
/// throws up a tab showing the old viewer.
///
/// The obvious fix — put a no-op `open` first on the child's PATH — does
/// nothing on macOS, and this was measured rather than assumed: the `open`
/// crate's macOS backend is `Command::new("/usr/bin/open")`, an absolute
/// path that PATH cannot shadow. The shim was in place and Chrome still went
/// from six tabs to seven, the new one pointing at `127.0.0.1:7995`. A
/// suppression that looks plausible and does nothing is worse than none,
/// because nobody goes back to check it.
///
/// So the guard is chosen per platform, and which rung is in use is reported
/// rather than assumed to have worked.
#[derive(Debug, Clone)]
pub enum BrowserGuard {
    /// The binary grew a `--no-open` flag. Preferred whenever present: it is
    /// the child agreeing not to, rather than the parent preventing it.
    Flag,
    /// macOS: run the child under a sandbox profile that denies exactly one
    /// thing — executing `/usr/bin/open`. The crate ignores the failed
    /// launch (`let _ = open::that_detached(..)`), so the server carries on.
    Sandbox(PathBuf),
    /// Other unix: the PATH shim genuinely works, because there the crate
    /// resolves `xdg-open`/`gio`/`gnome-open` through PATH.
    PathShim(String),
    /// Nothing available. Tabs will open, and we say so loudly instead of
    /// leaving it to be discovered.
    None,
}

impl BrowserGuard {
    pub fn describe(&self) -> &'static str {
        match self {
            BrowserGuard::Flag => "the server's own --no-open flag",
            BrowserGuard::Sandbox(_) => "a sandbox profile denying /usr/bin/open",
            BrowserGuard::PathShim(_) => "a no-op `open` first on PATH",
            BrowserGuard::None => "NOTHING — child servers will open browser tabs",
        }
    }
}

/// Pick the strongest guard this machine and this binary support.
pub fn choose_browser_guard(cache_dir: &Path) -> BrowserGuard {
    if serve_supports_no_open() {
        return BrowserGuard::Flag;
    }
    if cfg!(target_os = "macos") {
        if let Some(profile) = write_sandbox_profile(cache_dir) {
            if which("sandbox-exec") {
                return BrowserGuard::Sandbox(profile);
            }
        }
        // The PATH shim is known not to work here, so do not pretend.
        return BrowserGuard::None;
    }
    match path_shim(cache_dir) {
        Some(p) => BrowserGuard::PathShim(p),
        None => BrowserGuard::None,
    }
}

fn which(bin: &str) -> bool {
    std::process::Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ask the binary whether it takes `--no-open`, so a future git-lex that
/// grows the flag is used properly without changing anything here.
fn serve_supports_no_open() -> bool {
    std::process::Command::new("git-lex-serve")
        .arg("viz")
        .arg("--help")
        .output()
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr);
            text.contains("--no-open")
        })
        .unwrap_or(false)
}

/// `(allow default)` then one deny: the child keeps every other capability it
/// had, including binding its port and reading the store. A broad sandbox
/// would be a much bigger promise than "do not open a browser".
fn write_sandbox_profile(cache_dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(cache_dir).ok()?;
    let p = cache_dir.join("no-browser.sb");
    std::fs::write(
        &p,
        "(version 1)\n\
         ;; git-lex-ui: the child server opens a browser tab on startup and has\n\
         ;; no flag to disable it. Deny that one exec and nothing else.\n\
         (allow default)\n\
         (deny process-exec* (literal \"/usr/bin/open\"))\n",
    )
    .ok()?;
    Some(p)
}

fn path_shim(cache_dir: &Path) -> Option<String> {
    let shim_dir = cache_dir.join("shim");
    std::fs::create_dir_all(&shim_dir).ok()?;
    let shim = shim_dir.join("open");
    std::fs::write(&shim, "#!/bin/sh\n# git-lex-ui: swallow the browser launch.\nexit 0\n").ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    let existing = std::env::var("PATH").unwrap_or_default();
    Some(format!("{}:{}", shim_dir.display(), existing))
}


