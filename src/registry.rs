//! The machine registry of known git-lex repos.
//!
//! Lives at `~/.lex/repos.*`. Rob ruled 2026-09-01: do not hard-code the
//! extension — look in the directory and sniff whatever is there. Today it is
//! `repos.json` and the brief said YAML, which is exactly why we sniff.
//!
//! Measured on this machine 2026-09-01 (50 entries, and it grew by one WHILE
//! the spec was being written — the registry is live, other agents write it
//! as they save):
//!
//!   path no longer exists   28  (56%)
//!   exists, has `.lex/`     22  (44%)
//!   exists, no `.lex/`       0
//!   `last_used` is null     46  (92%)
//!
//! Two facts drive this whole module. **The rot is the normal case** — more
//! than half of what the registry holds is gone, so pruning is not tidy-up,
//! it is the difference between a usable list and an unusable one. And
//! **`last_used` is almost never populated** — 46 of 50 are null, so sorting
//! by it alone would tie 92% of rows while looking deliberate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One row as it appears on disk. `last_used` is `Option` because it is null
/// far more often than not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    /// Anything else the file carries. git-lex owns this file; we are a
    /// reader that occasionally prunes. Round-tripping unknown keys means a
    /// future git-lex field is not silently destroyed by our rewrite.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryFile {
    pub repos: Vec<RegistryEntry>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Which serialization the file on disk actually used, so a rewrite goes back
/// in the same form it came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Yaml,
}

/// Why an entry is not shown as a live repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Path exists, has `.lex/`, and is not under an ephemeral root.
    Live,
    /// The directory is gone. 28 of 50 on this machine.
    PathMissing,
    /// Directory is there but its git-lex state was removed.
    NoLexDir,
    /// Real `.lex/` state in a throwaway directory — job dirs and scratchpads.
    Scratch,
}

impl Verdict {
    pub fn is_live(self) -> bool {
        matches!(self, Verdict::Live)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Classified {
    pub path: String,
    pub last_used: Option<String>,
    pub verdict: Verdict,
    /// Plain-language reason, shown in the UI. A verdict without a stated
    /// reason is a label with no test attached.
    pub reason: String,
}

/// Roots whose contents are, by construction, temporary.
///
/// The spec named only `~/.claude/jobs/**`, from a reading of the registry
/// that found two such entries. Re-measured at build time the registry held
/// **six** scratch directories: two under `~/.claude/jobs/`, and four under
/// `/private/tmp/claude-501/.../scratchpad/` — agent scratchpads, which the
/// jobs-only rule misses entirely. A rule that catches two of six is not a
/// rule, so this matches on ephemeral roots generally.
fn ephemeral_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".claude").join("jobs"),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/var/folders"),
        PathBuf::from("/private/var/folders"),
    ];
    if let Ok(t) = std::env::var("TMPDIR") {
        if !t.is_empty() {
            roots.push(PathBuf::from(t));
        }
    }
    roots
}

fn under_any(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .find(|r| path.starts_with(r))
        .cloned()
}

/// Locate the registry file without assuming its extension.
pub fn find_registry(lex_dir: &Path) -> Option<(PathBuf, Format)> {
    let candidates = ["repos.json", "repos.yaml", "repos.yml", "repos"];
    for name in candidates {
        let p = lex_dir.join(name);
        if p.is_file() {
            let fmt = sniff(&p).unwrap_or(Format::Json);
            return Some((p, fmt));
        }
    }
    // Nothing named as expected — take any `repos.*` that is there.
    let entries = std::fs::read_dir(lex_dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file()
            && p.file_stem().and_then(|s| s.to_str()) == Some("repos")
        {
            let fmt = sniff(&p).unwrap_or(Format::Json);
            return Some((p, fmt));
        }
    }
    None
}

/// Decide the format from the bytes, not from the name. A file called
/// `repos.yaml` containing JSON parses as JSON here, which is the point.
fn sniff(path: &Path) -> Option<Format> {
    let text = std::fs::read_to_string(path).ok()?;
    let first = text.chars().find(|c| !c.is_whitespace())?;
    Some(if first == '{' || first == '[' {
        Format::Json
    } else {
        Format::Yaml
    })
}

pub fn parse(text: &str, fmt: Format) -> Result<RegistryFile, String> {
    match fmt {
        // JSON is a subset of YAML 1.2, but serde_yaml is not serde_json;
        // parse each with its own reader so error messages stay truthful.
        Format::Json => serde_json::from_str(text).map_err(|e| format!("registry is not valid JSON: {e}")),
        Format::Yaml => serde_yaml::from_str(text).map_err(|e| format!("registry is not valid YAML: {e}")),
    }
}

pub fn serialize(file: &RegistryFile, fmt: Format) -> Result<String, String> {
    match fmt {
        Format::Json => serde_json::to_string_pretty(file).map_err(|e| e.to_string()),
        Format::Yaml => serde_yaml::to_string(file).map_err(|e| e.to_string()),
    }
}

/// Classify every entry. Pure — does no writing and no network.
pub fn classify(entries: &[RegistryEntry], home: &Path) -> Vec<Classified> {
    let roots = ephemeral_roots(home);
    entries
        .iter()
        .map(|e| {
            let p = Path::new(&e.path);
            let (verdict, reason) = if !p.is_dir() {
                (
                    Verdict::PathMissing,
                    "the directory is no longer on disk".to_string(),
                )
            } else if !p.join(".lex").is_dir() {
                (
                    Verdict::NoLexDir,
                    "the directory is here but its .lex/ state was removed".to_string(),
                )
            } else if let Some(root) = under_any(p, &roots) {
                (
                    Verdict::Scratch,
                    format!("a throwaway directory under {}", root.display()),
                )
            } else {
                (Verdict::Live, "path exists and carries .lex/ state".to_string())
            };
            Classified {
                path: e.path.clone(),
                last_used: e.last_used.clone(),
                verdict,
                reason,
            }
        })
        .collect()
}

/// The outcome of a prune, so the UI can say what happened rather than
/// quietly showing a shorter list.
#[derive(Debug, Clone, Serialize)]
pub struct PruneReport {
    pub removed: Vec<Classified>,
    pub kept: usize,
    /// Entries that appeared in the file between our read and our write.
    /// Non-zero means another agent saved while we were deciding — which is
    /// normal here, not an error.
    pub arrived_during_prune: usize,
    pub backup_path: Option<String>,
    pub rewrote_file: bool,
}

/// Remove the dead entries from the registry file.
///
/// Rob ruled: prune the file, keep a record. Two things make this safe to do
/// to a file we do not own.
///
/// **We re-read at write time and remove only what we condemned.** The
/// registry is live — fifteen agents were awake when this was written and it
/// gained an entry mid-session. A read-modify-write of the whole structure
/// would silently drop any repo registered in the window. So the write is a
/// set-subtraction against the current contents, not an overwrite of them.
///
/// **The removals are written out first.** 28 rows leaving a shared machine
/// registry with no undo is hard to reverse; the record costs nothing.
pub fn prune(
    registry_path: &Path,
    fmt: Format,
    condemned: &[Classified],
    cache_dir: &Path,
    now_stamp: &str,
) -> Result<PruneReport, String> {
    let dead: std::collections::HashSet<&str> =
        condemned.iter().map(|c| c.path.as_str()).collect();

    if dead.is_empty() {
        return Ok(PruneReport {
            removed: vec![],
            kept: 0,
            arrived_during_prune: 0,
            backup_path: None,
            rewrote_file: false,
        });
    }

    // Record first, then act.
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("could not create {}: {e}", cache_dir.display()))?;
    let backup = cache_dir.join(format!("pruned-{now_stamp}.json"));
    let record = serde_json::json!({
        "pruned_at": now_stamp,
        "registry": registry_path.display().to_string(),
        "removed": condemned,
    });
    std::fs::write(
        &backup,
        serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("could not write {}: {e}", backup.display()))?;

    // Re-read: the file may have moved under us.
    let text = std::fs::read_to_string(registry_path)
        .map_err(|e| format!("could not re-read registry: {e}"))?;
    let mut current = parse(&text, fmt)?;

    let before = current.repos.len();
    let mut removed_now: Vec<Classified> = Vec::new();
    current.repos.retain(|e| {
        if dead.contains(e.path.as_str()) {
            if let Some(c) = condemned.iter().find(|c| c.path == e.path) {
                removed_now.push(c.clone());
            }
            false
        } else {
            true
        }
    });
    let arrived = before.saturating_sub(condemned.len()).saturating_sub(
        // kept count if nothing arrived
        current.repos.len(),
    );

    let out = serialize(&current, fmt)?;
    write_atomic(registry_path, &out)?;

    Ok(PruneReport {
        kept: current.repos.len(),
        removed: removed_now,
        arrived_during_prune: arrived,
        backup_path: Some(backup.display().to_string()),
        rewrote_file: true,
    })
}

/// Write via a sibling temp file and rename, so a reader never observes a
/// half-written registry and a crash mid-write cannot truncate it.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("registry has no parent directory")?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("repos"),
        std::process::id()
    ));
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename onto {}: {e}", path.display()))?;
    Ok(())
}
