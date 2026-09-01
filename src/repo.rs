//! Probing a repo for everything the front door needs *before* any server
//! boots.
//!
//! The whole point of the picker is that it paints fast. Booting a `git lex
//! serve` per repo to find out what each repo is called would defeat that —
//! lUX alone takes ~3 seconds to build its working-tree view. Everything
//! here comes from two cheap local reads: `.lex/repo.yml` and `git`.
//!
//! `.lex/repo.yml` is the identity card and it is authoritative:
//!
//!   name: W3BL0RD        agent_name: w3bl0rd      kit: soul
//!   genesis_sha: e3d71e7f0e022e54d3cdfb3100862f21f10913ad
//!
//! Verified 2026-09-01: `genesis_sha` is exactly `git rev-list
//! --max-parents=0 HEAD`, and it is the same value the running server reports
//! as `gl:genesisSha` in its `repo` named graph. That equality is what lets
//! this module hand out an identity that a live server can be checked
//! against without either side trusting the other's memory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// `.lex/repo.yml` as git-lex writes it. Every field is optional: this file
/// is written by a tool that has changed shape before and will again, and a
/// missing `name` should cost us a nicer label, not the whole row.
#[derive(Debug, Clone, Default, Deserialize)]
struct RepoYml {
    name: Option<String>,
    agent_name: Option<String>,
    kit: Option<String>,
    created: Option<serde_yaml::Value>,
    genesis_sha: Option<String>,
    #[serde(default)]
    optional_kits: Vec<String>,
}

/// Which clock produced the timestamp a row is sorted by.
///
/// This exists because 46 of 50 registry entries have `last_used: null`.
/// Sorting by `last_used` alone would tie 92% of rows at null while looking
/// decisive, so we fall back to the repo's own last commit — and then we have
/// two clocks in one column, which is only honest if the column says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecencySource {
    /// The registry's own `last_used`. 4 of 50 rows.
    LastUsed,
    /// The repo's most recent commit, because `last_used` was null.
    HeadCommit,
    /// Neither was available.
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoProbe {
    pub path: String,
    /// Permanent identity. `None` only if the repo has no commits at all.
    pub genesis_sha: Option<String>,
    /// Display name from repo.yml, falling back to the directory name.
    pub name: String,
    /// True when `name` is a real declared name rather than a directory guess.
    pub name_declared: bool,
    pub agent_name: Option<String>,
    pub kit: Option<String>,
    pub optional_kits: Vec<String>,
    pub head_sha: Option<String>,
    /// ISO-8601 of the most recent commit.
    pub head_time: Option<String>,
    pub commit_count: Option<u64>,
    /// The timestamp this row should sort by, and where it came from.
    pub recency: Option<String>,
    pub recency_source: RecencySource,
    /// Whether `.lex/www/` exists — the directory the per-repo server reads
    /// its frontend from on every request.
    pub has_www: bool,
    /// Non-fatal problems found while probing. Shown, never swallowed.
    pub warnings: Vec<String>,
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("(unnamed)")
        .to_string()
}

async fn git(path: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Probe one repo. Shells out to `git` rather than linking libgit2: these are
/// a handful of one-shot metadata reads across at most a few dozen repos, run
/// concurrently, and `git` is guaranteed present in a tool that only exists
/// to look at git repos. Linking a vendored libgit2 would buy nothing here
/// and cost every build.
pub async fn probe(path: &Path, last_used: Option<String>) -> RepoProbe {
    let mut warnings = Vec::new();

    let yml: RepoYml = match std::fs::read_to_string(path.join(".lex/repo.yml")) {
        Ok(text) => match serde_yaml::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!(".lex/repo.yml did not parse: {e}"));
                RepoYml::default()
            }
        },
        Err(_) => {
            warnings.push(".lex/repo.yml is missing".to_string());
            RepoYml::default()
        }
    };

    let (head_sha, head_time, root_sha, count) = tokio::join!(
        git(path, &["rev-parse", "HEAD"]),
        git(path, &["log", "-1", "--format=%cI"]),
        git(path, &["rev-list", "--max-parents=0", "HEAD"]),
        git(path, &["rev-list", "--count", "HEAD"]),
    );

    // A root-commit list can hold more than one line if histories were
    // grafted together. Take the last, which is the earliest.
    let root_sha = root_sha.and_then(|s| s.lines().last().map(|l| l.trim().to_string()));

    // Cross-check the declared identity against the repository itself. This
    // is the one place the two can disagree, and if they do, the file is
    // wrong and the commit graph is right.
    let genesis_sha = match (yml.genesis_sha.clone(), root_sha.clone()) {
        (Some(declared), Some(actual)) if declared != actual => {
            warnings.push(format!(
                "repo.yml declares genesis {} but the root commit is {} — trusting the commit graph",
                &declared[..declared.len().min(8)],
                &actual[..actual.len().min(8)]
            ));
            Some(actual)
        }
        (Some(declared), _) => Some(declared),
        (None, actual) => actual,
    };

    let (name, name_declared) = match yml.name.clone() {
        Some(n) if !n.trim().is_empty() => (n, true),
        _ => (dir_name(path), false),
    };

    let (recency, recency_source) = match (last_used.clone(), head_time.clone()) {
        (Some(lu), _) => (Some(lu), RecencySource::LastUsed),
        (None, Some(ht)) => (Some(ht), RecencySource::HeadCommit),
        (None, None) => (None, RecencySource::Unknown),
    };

    RepoProbe {
        path: path.display().to_string(),
        genesis_sha,
        name,
        name_declared,
        agent_name: yml.agent_name,
        kit: yml.kit,
        optional_kits: yml.optional_kits,
        head_sha,
        head_time,
        commit_count: count.and_then(|c| c.parse().ok()),
        recency,
        recency_source,
        has_www: path.join(".lex/www").is_dir(),
        warnings,
    }
}

/// Probe many repos at once. The picker's first paint waits on this, so it
/// fans out rather than walking.
pub async fn probe_all(rows: Vec<(PathBuf, Option<String>)>) -> Vec<RepoProbe> {
    let tasks: Vec<_> = rows
        .into_iter()
        .map(|(p, lu)| tokio::spawn(async move { probe(&p, lu).await }))
        .collect();
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        if let Ok(p) = t.await {
            out.push(p);
        }
    }
    out
}
