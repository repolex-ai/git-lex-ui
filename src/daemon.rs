//! Talking to `gitlexd`, the one store daemon on the machine.
//!
//! **This replaces a supervisor that started one `git-lex-serve sparql`
//! process per repo.** That command and its binary were removed from git-lex
//! on 2026-09-22 (merge f428987, @w4r3z), and with them went every problem
//! the supervisor existed to solve: port allocation, port walking, adopting
//! orphans left by a previous run, killing children on shutdown, and the
//! identity check that caught a server quietly serving the wrong repo.
//!
//! What replaces it is one process for the whole machine, on a fixed port,
//! holding every registered repo's store and addressing each one by its
//! genesis sha. The front door used to translate a genesis sha into a port
//! because a port is a place and a genesis sha is a name; now the daemon
//! takes the name directly and the translation has nowhere left to go wrong.
//!
//! Two things about starting it, both from git-lex's own docs
//! (`docs/using/gitlexd.md`):
//!
//! - **The port is the lock.** A second `gitlexd` told to start finds 7880
//!   held and exits by itself. So the recovery from "nothing is answering" is
//!   simply to spawn one and wait — there is no race to lose, and no state to
//!   reconcile if two arrive at once.
//! - **We are usually not the one who starts it.** `git lex query` starts it
//!   on demand, so on a machine where anyone has run git-lex today it is
//!   already up. This asks first and spawns only on a refused connection.

use serde::Serialize;

/// Where `gitlexd` lives. Fixed by git-lex, not negotiable, and that is the
/// point — a daemon that moved would need discovery, and discovery is what
/// the per-repo port table was.
pub const DAEMON_PORT: u16 = 7880;

/// How long to wait for a daemon we started to answer. Opening every store on
/// the machine is the slow part; it is seconds, not tens of seconds.
const START_TIMEOUT_MS: u128 = 20_000;

/// One repo, as the daemon reports it. This is `GET /souls`, which is the
/// answer to "what is held, and how current is each one" in a single call —
/// the question the old supervisor answered with a health poll per child.
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
pub struct Soul {
    pub genesis: String,
    pub name: Option<String>,
    pub path: String,
    /// The commit the store has been built up to. This is the LIVE answer to
    /// how far a graph has fallen behind, from the process that did the
    /// building. Everything else available is a trace left on disk, and a
    /// trace has to be interpreted — which is where the reading before this
    /// one went wrong. See `GraphFreshness::from_daemon`.
    #[serde(default)]
    pub synced_to: Option<String>,
    #[serde(default)]
    pub syncing: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub open_error: Option<String>,
    #[serde(default)]
    pub last_sync_ms: Option<u64>,
    #[serde(default)]
    pub syncs: Option<u64>,
}

/// The daemon's own `/health`.
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
pub struct Health {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub souls: Option<usize>,
    #[serde(default)]
    pub syncing: Option<usize>,
    #[serde(default)]
    pub uptime_secs: Option<u64>,
    #[serde(default)]
    pub version: Option<String>,
}

/// What the front door reports to the page about the feed as a whole.
///
/// There is one of these now, where there used to be one row per repo. A page
/// that cannot draw has exactly one thing to check, and it is named here
/// rather than inferred from a dozen unreachable children.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatus {
    pub reachable: bool,
    pub port: u16,
    pub health: Option<Health>,
    /// Every soul the daemon holds, so the page can mark the repos it can
    /// actually draw. Empty when the daemon is not answering — which is not
    /// the same as the daemon holding nothing, so `reachable` carries that.
    pub souls: Vec<Soul>,
    pub message: Option<String>,
}

pub struct Daemon {
    http: reqwest::Client,
    port: u16,
}

impl Daemon {
    pub fn new(http: reqwest::Client, port: u16) -> Self {
        Self { http, port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The SPARQL base for one soul. Every query in this tool goes through
    /// here, addressed by genesis sha.
    pub fn soul_base(&self, genesis: &str) -> String {
        format!("{}/soul/{genesis}", self.base())
    }

    pub async fn health(&self) -> Result<Health, String> {
        let r = self
            .http
            .get(format!("{}/health", self.base()))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        r.json().await.map_err(|e| e.to_string())
    }

    pub async fn souls(&self) -> Result<Vec<Soul>, String> {
        let r = self
            .http
            .get(format!("{}/souls", self.base()))
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("the daemon refused /souls ({})", r.status()));
        }
        #[derive(serde::Deserialize)]
        struct Envelope {
            souls: Vec<Soul>,
        }
        let e: Envelope = r.json().await.map_err(|e| format!("bad /souls reply: {e}"))?;
        Ok(e.souls)
    }

    /// Make sure something is answering on the daemon port, starting one if
    /// not, and report what we found.
    ///
    /// Spawning is safe to do speculatively — the port is the lock, so a
    /// second daemon exits on its own. It is still not done speculatively,
    /// because "I started it" and "it was already there" are different facts
    /// and the startup line should say which.
    pub async fn ensure(&self) -> DaemonStatus {
        if let Ok(h) = self.health().await {
            return self.status_with(h, None).await;
        }

        let spawned = tokio::process::Command::new("gitlexd")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(e) = spawned {
            return DaemonStatus {
                reachable: false,
                port: self.port,
                health: None,
                souls: vec![],
                message: Some(format!(
                    "nothing is answering on port {} and `gitlexd` could not be started: {e}. \
                     It ships with git-lex — try `cargo install --path .` in the git-lex repo, \
                     or run `git lex query \"ASK {{}}\"` in any repo to start one.",
                    self.port
                )),
            };
        }

        let began = now_ms();
        loop {
            if let Ok(h) = self.health().await {
                return self.status_with(h, Some("started gitlexd".to_string())).await;
            }
            if now_ms().saturating_sub(began) > START_TIMEOUT_MS {
                return DaemonStatus {
                    reachable: false,
                    port: self.port,
                    health: None,
                    souls: vec![],
                    message: Some(format!(
                        "started `gitlexd` but it did not answer on port {} within {} seconds",
                        self.port,
                        START_TIMEOUT_MS / 1000
                    )),
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    /// Current state without trying to start anything. Asked on a timer, so a
    /// page left open finds out when the daemon dies.
    pub async fn status(&self) -> DaemonStatus {
        match self.health().await {
            Ok(h) => self.status_with(h, None).await,
            Err(e) => DaemonStatus {
                reachable: false,
                port: self.port,
                health: None,
                souls: vec![],
                message: Some(format!("gitlexd is not answering on port {}: {e}", self.port)),
            },
        }
    }

    async fn status_with(&self, health: Health, note: Option<String>) -> DaemonStatus {
        let (souls, message) = match self.souls().await {
            Ok(s) => (s, note),
            Err(e) => (vec![], Some(e)),
        };
        DaemonStatus {
            reachable: true,
            port: self.port,
            health: Some(health),
            souls,
            message,
        }
    }

    /// Ask the daemon to sync one soul and wait for the result.
    ///
    /// `?wait=1` returns the state after the sync rather than a bare 202, so
    /// the front door does not have to poll for something the daemon already
    /// knows. lUX takes minutes, hence the long timeout.
    pub async fn sync(&self, genesis: &str) -> Result<Soul, String> {
        let r = self
            .http
            .post(format!("{}/sync?wait=1", self.soul_base(genesis)))
            .timeout(std::time::Duration::from_secs(900))
            .send()
            .await
            .map_err(|e| format!("the sync request failed: {e}"))?;
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() {
            // The daemon's errors are `{"error": "..."}`. Keep the message
            // verbatim — a sync failure is usually a real fact about the repo,
            // and paraphrasing it loses the fix.
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .unwrap_or_else(|| body.trim().to_string());
            return Err(if detail.is_empty() {
                format!("the sync was refused ({status})")
            } else {
                detail
            });
        }
        serde_json::from_str::<Soul>(&body)
            .map_err(|e| format!("the sync finished but its reply did not parse: {e}"))
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The daemon reports a soul with fields this tool did not ask for, and
    /// will gain more. Every optional column is defaulted so a new field on
    /// git-lex's side never turns into "no souls are held" over here.
    #[test]
    fn a_souls_reply_parses_with_only_the_fields_we_require() {
        let v: Soul = serde_json::from_str(
            r#"{"genesis":"abc","name":"W3BL0RD","path":"/x","future_field":7}"#,
        )
        .expect("unknown fields must not fail the parse");
        assert_eq!(v.genesis, "abc");
        assert!(v.synced_to.is_none());
        assert!(!v.syncing);
    }

    /// A soul addressed by name, never by a port — the one property from the
    /// supervisor era worth carrying forward, now for free.
    #[test]
    fn a_soul_is_addressed_by_its_genesis_sha() {
        let d = Daemon::new(reqwest::Client::new(), 7880);
        assert_eq!(
            d.soul_base("e3d71e7f0e022e54d3cdfb3100862f21f10913ad"),
            "http://127.0.0.1:7880/soul/e3d71e7f0e022e54d3cdfb3100862f21f10913ad"
        );
    }
}
