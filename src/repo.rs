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

/// What kind of git-lex repo this is.
///
/// **git-lex's base case is a plain repository of markdown files.** The soul
/// kit is one thing you can install on top; it is not what git-lex is. This
/// front door was written against a machine holding 20 souls and had quietly
/// started treating "repo" and "soul" as the same word. Caught by @goodlux
/// asked for the list to be segmented, which is right, and cheap, and stops
/// the assumption spreading further into the UI.
///
/// Measured across the 24 registered repos on 2026-09-03: 20 soul, 2 squad,
/// 1 autoknow, 1 plain (git-lex's own repo). Two independent signals — the
/// `kit` field in `.lex/repo.yml`, and whether a `Soul/` directory or
/// `SOUL.md` exists on disk — agreed on **all 24**, so the declared kit is
/// trustworthy here and the filesystem check is a cross-check rather than a
/// fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "family", rename_all = "kebab-case")]
pub enum RepoFamily {
    /// Runs the soul kit: journals, notes, pursuits, an identity document.
    Soul,
    /// Runs some other kit. Named rather than lumped, because "not a soul"
    /// is not the same fact as "no kit" and a squad repo is neither.
    Kitted { kit: String },
    /// The base case: markdown in git, with git-lex tracking it. No kit.
    Plain,
}

impl RepoFamily {
    /// Classify from the declared kit, cross-checked against the tree.
    ///
    /// The kit string has two spellings in the wild — bare `soul` (6 repos)
    /// and the full `repolex-ai/git-lex-kit-soul` (14) — so this matches on
    /// the trailing segment rather than the whole value. A match on the
    /// literal `"soul"` alone would have mis-filed 14 of 20 souls as some
    /// other kit, which is precisely the kind of quiet miscount that makes a
    /// segmented list worse than no segmentation at all.
    fn classify(kit: Option<&str>, root: &Path) -> Self {
        let looks_soul_on_disk = root.join("Soul").is_dir() || root.join("SOUL.md").is_file();
        match kit {
            None => {
                // A tree that looks like a soul but declares no kit is still
                // a soul to a reader; trust the evidence over the absence.
                if looks_soul_on_disk {
                    RepoFamily::Soul
                } else {
                    RepoFamily::Plain
                }
            }
            Some(k) => {
                let last = k.rsplit('-').next().unwrap_or(k);
                if last.eq_ignore_ascii_case("soul") || looks_soul_on_disk {
                    RepoFamily::Soul
                } else {
                    RepoFamily::Kitted {
                        kit: last.to_string(),
                    }
                }
            }
        }
    }
}

/// How far the persisted graph has fallen behind the repository.
///
/// Measured 2026-09-03 across the 24 souls in the registry: **13 of the 15
/// that have ever been synced were behind**, by 74 commits in total, worst
/// case lUX at 27. So this is the normal state of a soul, not an alarm — and
/// nothing anywhere said so, because `git lex save` advances the repo and
/// only `git lex sync` advances the graph.
///
/// The probe is an `ls`. `git lex sync` writes
/// `.lex/_ignore/spine/<sha>.spine.tsv`, **named for the commit it was built
/// at**, so the graph's position is readable from a filename without opening
/// the store, starting a server, or perturbing anything. Credit to
/// @w3bl0rd-web, who found it while reproducing the staleness bug.
///
/// The variants are deliberately not collapsible to a boolean: 3 souls have
/// an oxigraph store but no spine file at all, and 6 have neither. "I cannot
/// tell" must never render as "current" — zero is the reassuring answer and
/// it is exactly the fallback that produced four separate wrong captions on
/// 2026-08-27.
/// **Superseded as the primary source on 2026-09-22 — see `from_daemon`.**
/// The spine reading below is now the fallback for a repo `gitlexd` does not
/// hold, and everything it says about markers is still true of it.
///
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum GraphFreshness {
    /// The spine's sha is HEAD. The graph describes today.
    Current { sha: String },
    /// The spine names an ancestor of HEAD. `commits` is `None` when the
    /// count could not be taken (a spine naming a sha this repo does not
    /// contain — a rewritten history, or a spine copied in from elsewhere).
    Behind {
        sha: String,
        commits: Option<u64>,
    },
    /// A store exists but no spine names its position. Cannot be placed from
    /// the filesystem alone; ask the running server for its newest commit.
    Unplaceable,
    /// **The store was written AFTER the spine that claims to describe it.**
    ///
    /// The spine is a position marker, not the position — and the two can
    /// disagree. Found on lUX 2026-09-16: its spine said 15 commits behind,
    /// its store had actually been rebuilt three days later to 8 behind, and
    /// the rebuild had never produced a `now` view at all, so the soul could
    /// not be drawn. A sync had been interrupted between writing the big
    /// graphs and finishing the job.
    ///
    /// A number here would have been wrong in both directions at once:
    /// pessimistic about the distance, and silent about the only fact that
    /// mattered. So this reports the disagreement instead of a figure.
    SpineStale {
        /// What the spine claims, kept because it is still a clue.
        spine_sha: String,
    },
    /// No store at all. This repo has never been synced.
    NeverSynced,
}

impl GraphFreshness {
    /// The graph's position as the process that WRITES it reports it, rather
    /// than as the filename left behind by a process that used to.
    ///
    /// This closed a false alarm that had been open for two days, and the
    /// reason is not the one it looked like.
    ///
    /// The reading below this one compares the spine file against the store's
    /// newest data file and flags a store written after its own marker. The
    /// spine is fine: git-lex refreshes it at the tail of every sync, the
    /// daemon's syncs included, and the sha in its name is always truthful.
    /// **What is not fine is inferring "the graph moved" from "a store file
    /// changed", because merely OPENING a RocksDB store rewrites it** — the
    /// bookkeeping files every time, and `.sst` data files whenever the open
    /// triggers a compaction. `gitlexd` opens every store on the machine when
    /// it starts, so from 2026-09-22 this fired on all of them: six repos
    /// carried a warning triangle while the daemon reported them synced to
    /// exactly HEAD. On 4RX the spine named HEAD, and every file in the store
    /// was stamped 14:01 — the second the daemon started.
    ///
    /// This is the second time the same door has been walked through. On
    /// 2026-09-16 the check compared the store FOLDER's timestamp, and opening
    /// a store bumped it; the fix was to look only at `.sst` files, on the
    /// belief that data files change when facts change. They also change when
    /// nobody changes anything. Narrowing an over-broad signal is not the same
    /// as establishing that what is left means what you want it to mean, and a
    /// detector that responds to being LOOKED AT is the flinch test failing on
    /// the instrument itself.
    ///
    /// So the answer is not a narrower proxy. It is to ask the process that
    /// does the writing what it wrote, which is what this does. Credit to
    /// @w4r3z, who read a claim of mine that the daemon had stopped
    /// maintaining the spine and showed it was false in one line.
    pub fn from_daemon(synced_to: Option<&str>, head: Option<&str>, syncing: bool) -> Self {
        match (synced_to, head) {
            // Mid-sync, so any distance is about to be wrong. Placed as
            // unplaceable rather than guessed at.
            _ if syncing => GraphFreshness::Unplaceable,
            (None, _) => GraphFreshness::NeverSynced,
            (Some(sha), Some(h)) if sha == h => GraphFreshness::Current { sha: sha.to_string() },
            // Behind, and the distance is counted by the caller, which has
            // the repo to count it in. `None` is "I cannot tell", never zero.
            (Some(sha), _) => GraphFreshness::Behind { sha: sha.to_string(), commits: None },
        }
    }
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
    /// Soul, some other kit, or plain markdown. The list segments on this.
    pub family: RepoFamily,
    pub optional_kits: Vec<String>,
    pub head_sha: Option<String>,
    /// ISO-8601 of the most recent commit.
    pub head_time: Option<String>,
    pub commit_count: Option<u64>,
    /// The timestamp this row should sort by, and where it came from.
    pub recency: Option<String>,
    pub recency_source: RecencySource,
    /// How far the persisted graph trails the repo. Read from the spine
    /// filename; never inferred, never defaulted to "fine".
    pub graph: GraphFreshness,
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

/// Read the graph's position from the spine filename, and how far HEAD has
/// run ahead of it.
///
/// Deliberately does not open the store. The whole value of this probe is
/// that it costs one directory listing per repo and is safe to run against
/// every soul on the machine at paint time, including ones nothing is
/// serving.
async fn graph_freshness(path: &Path, head: Option<&str>) -> GraphFreshness {
    let spine_dir = path.join(".lex/_ignore/spine");
    let store = path.join(".lex/_ignore/oxigraph");

    // Collect the shas the spine directory names. Normally exactly one; take
    // the newest by mtime if a repo has accumulated several, since the spine
    // is a position marker and the latest one is the position.
    let mut spines: Vec<(std::time::SystemTime, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&spine_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(sha) = name.strip_suffix(".spine.tsv") else {
                continue;
            };
            if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let t = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            spines.push((t, sha.to_string()));
        }
    }
    spines.sort_by(|a, b| b.0.cmp(&a.0));

    let Some((spine_time, sha)) = spines.into_iter().next() else {
        return if store.is_dir() {
            GraphFreshness::Unplaceable
        } else {
            GraphFreshness::NeverSynced
        };
    };

    // Does the spine actually describe THIS store? `git lex sync` writes the
    // store and then the spine, so store DATA written after the spine means
    // the spine's position is a claim about a store that has since changed.
    //
    // Compared against the newest `.sst` — the database's data files — and
    // NOT the store directory's own timestamp. The first version used the
    // directory, and it flagged four souls on 2026-09-17, two of them
    // falsely: the database rewrites its MANIFEST, OPTIONS and LOG on every
    // OPEN, so any read — including this front door serving the graph —
    // bumped the directory without changing a single fact. On 4RX the
    // directory was 12.5 hours newer than the spine and the newest data file
    // was the same second. A detector that fires because it was looked at
    // is the flinch test failing on the instrument itself.
    //
    // A minute of slack: the two writes are seconds apart in a healthy sync.
    let newest_data = std::fs::read_dir(&store).ok().and_then(|rd| {
        rd.flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".sst"))
            .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
            .max()
    });
    if let Some(data_time) = newest_data {
        if let Ok(gap) = data_time.duration_since(spine_time) {
            if gap.as_secs() > 60 {
                return GraphFreshness::SpineStale { spine_sha: sha };
            }
        }
    }

    match head {
        Some(h) if h == sha => GraphFreshness::Current { sha },
        Some(h) => {
            let commits = git(path, &["rev-list", "--count", &format!("{sha}..{h}")])
                .await
                .and_then(|c| c.parse::<u64>().ok());
            GraphFreshness::Behind { sha, commits }
        }
        // No HEAD to compare against: we know where the graph sits but not
        // whether that is current. Saying "current" here would be a guess.
        None => GraphFreshness::Behind { sha, commits: None },
    }
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

    let graph = graph_freshness(path, head_sha.as_deref()).await;
    let family = RepoFamily::classify(yml.kit.as_deref(), path);

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
        family,
        optional_kits: yml.optional_kits,
        graph,
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

#[cfg(test)]
mod tests {
    use super::GraphFreshness as F;

    /// The false alarm this reading exists to end. Six repos carried a
    /// warning triangle on 2026-09-22 while the daemon had them synced to
    /// exactly HEAD — not because the spine had gone unwritten, but because
    /// the daemon had OPENED every store, and an open leaves the same traces
    /// on disk that real work does.
    #[test]
    fn a_store_the_daemon_synced_to_head_is_current_however_old_its_spine_is() {
        let f = F::from_daemon(Some("abc123"), Some("abc123"), false);
        assert!(matches!(f, F::Current { .. }), "got {f:?}");
    }

    /// Behind is reported with no number here: the distance is counted in the
    /// repo by the caller, and `None` means "not counted yet", never zero.
    #[test]
    fn a_store_short_of_head_is_behind_without_inventing_a_distance() {
        match F::from_daemon(Some("older"), Some("newer"), false) {
            F::Behind { sha, commits } => {
                assert_eq!(sha, "older");
                assert_eq!(commits, None);
            }
            other => panic!("got {other:?}"),
        }
    }

    /// A repo git-lex has never been run in. Distinct from behind: there is
    /// nothing to catch up, there is nothing at all.
    #[test]
    fn a_soul_the_daemon_has_never_synced_is_never_synced() {
        assert!(matches!(F::from_daemon(None, Some("abc"), false), F::NeverSynced));
    }

    /// Mid-sync, every distance is about to be wrong, so none is offered.
    /// "I cannot tell" must never render as "current" — that fallback is what
    /// produced four wrong captions on 2026-08-27.
    #[test]
    fn a_store_being_synced_right_now_is_not_called_current() {
        let f = F::from_daemon(Some("abc123"), Some("abc123"), true);
        assert!(matches!(f, F::Unplaceable), "got {f:?}");
    }

    use super::*;

    fn repo(dir: &Path) -> String {
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("git");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        for n in 0..3 {
            std::fs::write(dir.join(format!("f{n}")), "x").unwrap();
            run(&["add", "-A"]);
            run(&["commit", "-qm", &format!("c{n}")]);
        }
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn spine(dir: &Path, sha: &str) {
        let d = dir.join(".lex/_ignore/spine");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{sha}.spine.tsv")), "x").unwrap();
    }

    /// The four states must stay four. A soul with a store but no spine, and
    /// a soul with nothing at all, are both "I cannot answer that" — and the
    /// one thing neither may ever become is `Current`, because `Current` is
    /// the answer that stops someone looking.
    #[tokio::test]
    async fn unanswerable_is_never_reported_as_current() {
        let tmp = std::env::temp_dir().join(format!("glui-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let head = repo(&tmp);

        // No store, no spine.
        assert!(matches!(
            graph_freshness(&tmp, Some(&head)).await,
            GraphFreshness::NeverSynced
        ));

        // A store, but nothing naming its position.
        std::fs::create_dir_all(tmp.join(".lex/_ignore/oxigraph")).unwrap();
        assert!(matches!(
            graph_freshness(&tmp, Some(&head)).await,
            GraphFreshness::Unplaceable
        ));

        // A spine at HEAD is the only thing that earns `Current`.
        spine(&tmp, &head);
        assert!(matches!(
            graph_freshness(&tmp, Some(&head)).await,
            GraphFreshness::Current { .. }
        ));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// The base case must survive a machine that is 20/24 souls.
    ///
    /// The assumption @goodlux caught (2026-09-03): a front door written here
    /// treating "repo" and "soul" as synonyms, because on this machine they
    /// almost are. A plain markdown repo has to classify as plain even
    /// though nothing on this laptop looks like one except git-lex itself.
    #[test]
    fn plain_markdown_is_not_mistaken_for_a_soul() {
        let tmp = std::env::temp_dir().join(format!("glui-fam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // No kit, no Soul/ — the base case.
        assert_eq!(RepoFamily::classify(None, &tmp), RepoFamily::Plain);

        // Both spellings of the soul kit found in the wild. Matching the
        // literal "soul" alone would mis-file 14 of this machine's 20 souls.
        for k in ["soul", "repolex-ai/git-lex-kit-soul"] {
            assert_eq!(RepoFamily::classify(Some(k), &tmp), RepoFamily::Soul, "{k}");
        }

        // A different kit is neither a soul nor plain.
        assert_eq!(
            RepoFamily::classify(Some("repolex-ai/git-lex-kit-squad"), &tmp),
            RepoFamily::Kitted { kit: "squad".into() }
        );

        // Evidence on disk outweighs a missing declaration.
        std::fs::create_dir_all(tmp.join("Soul")).unwrap();
        assert_eq!(RepoFamily::classify(None, &tmp), RepoFamily::Soul);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// A spine older than its store must not report a position.
    ///
    /// This is lUX on 2026-09-16: the spine said 15 behind, the store had been
    /// rewritten three days later by a sync that never finished, and the soul
    /// could not be drawn at all. The spine's number was a claim about a store
    /// that no longer existed. The honest answer is the disagreement itself.
    #[tokio::test]
    async fn a_store_rewritten_after_its_spine_is_not_given_a_number() {
        let tmp = std::env::temp_dir().join(format!("glui-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let head = repo(&tmp);
        let store = tmp.join(".lex/_ignore/oxigraph");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("000001.sst"), "data").unwrap();
        // An open-only file, rewritten on every read. Must NOT count.
        std::fs::write(store.join("MANIFEST-000001"), "m").unwrap();

        // Spine written, then the store's DATA touched well after it.
        spine(&tmp, &head);
        let spine_file = tmp.join(format!(".lex/_ignore/spine/{head}.spine.tsv"));
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
        std::fs::File::options()
            .write(true)
            .open(&spine_file)
            .unwrap()
            .set_modified(old)
            .unwrap();

        match graph_freshness(&tmp, Some(&head)).await {
            GraphFreshness::SpineStale { spine_sha } => assert_eq!(spine_sha, head),
            other => panic!(
                "a spine three days older than its store must not be trusted, got {other:?} \
                 — note it would otherwise have read as CURRENT, since the spine names HEAD"
            ),
        }

        // A read bumps only the open-time files. With data older than the
        // spine, that must not fire — this is the false alarm the directory
        // timestamp raised on 4RX and TR1P.L3X.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let spine_newer = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&spine_file).unwrap()
            .set_modified(spine_newer).unwrap();
        std::fs::File::options().write(true).open(store.join("000001.sst")).unwrap()
            .set_modified(std::time::SystemTime::now()).unwrap();
        std::fs::File::options().write(true).open(store.join("MANIFEST-000001")).unwrap()
            .set_modified(later).unwrap();
        assert!(
            matches!(graph_freshness(&tmp, Some(&head)).await, GraphFreshness::Current { .. }),
            "opening the store must not make it look stale"
        );

        // And a spine written just after its store is still believed: the
        // check must not fire on a healthy sync.
        std::fs::File::options()
            .write(true)
            .open(&spine_file)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            graph_freshness(&tmp, Some(&head)).await,
            GraphFreshness::Current { .. }
        ));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// The count is the whole value of the marker, so it has to be a real
    /// distance, not a boolean dressed as a number. Two commits back must
    /// read as 2 — and a spine naming a sha this repo has never heard of
    /// must report `None` rather than 0, since 0 is indistinguishable from
    /// "up to date" at the glance this marker is designed for.
    #[tokio::test]
    async fn behind_carries_a_real_distance_and_admits_when_it_cannot() {
        let tmp = std::env::temp_dir().join(format!("glui-dist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let head = repo(&tmp);
        let older = String::from_utf8_lossy(
            &std::process::Command::new("git")
                .arg("-C")
                .arg(&tmp)
                .args(["rev-parse", "HEAD~2"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();

        spine(&tmp, &older);
        match graph_freshness(&tmp, Some(&head)).await {
            GraphFreshness::Behind { commits, sha } => {
                assert_eq!(commits, Some(2), "two commits back must read as 2");
                assert_eq!(sha, older);
            }
            other => panic!("expected Behind, got {other:?}"),
        }

        // A sha from nowhere: the distance is unknowable, and must say so.
        let _ = std::fs::remove_dir_all(tmp.join(".lex/_ignore/spine"));
        spine(&tmp, &"a".repeat(40));
        match graph_freshness(&tmp, Some(&head)).await {
            GraphFreshness::Behind { commits, .. } => {
                assert_eq!(commits, None, "an unreachable spine sha must not report 0")
            }
            other => panic!("expected Behind, got {other:?}"),
        }

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
