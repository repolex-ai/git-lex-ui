//! Serving the Whole Soul layout, and caching it against the commit it was
//! built from.
//!
//! Cache the expensive and derived, never the authoritative. The store is the
//! truth; this cache exists so a soul that has not changed does not pay to be
//! laid out twice.
//!
//! **Every cache entry carries the repo's identity and the HEAD it was built
//! from, and is thrown away when either changes.** A cached figure presented
//! as current is the defect that produced four separate bugs on 2026-08-27,
//! including an axis that showed an 18-day-old snapshot as "now". The
//! metadata says when it was built and whether it came from cache, so the
//! page can say so too.
//!
//! The cache is keyed on **genesis sha**, not on a hash of the path. A path
//! is where a repo is sitting today; the genesis sha is which repo it is. Key
//! on the path and every `mv` silently orphans the cache.

use crate::layout::{self, Layout, View};
use crate::sparql::SparqlClient;
use std::path::{Path, PathBuf};

/// Bumped whenever the *shape or values* of a computed layout change, not
/// just its inputs.
///
/// The cache is keyed by HEAD, which correctly invalidates when the repo
/// moves — but a change to the layout code itself moves nothing, so every
/// already-cached soul would keep serving the old geometry with no way to
/// tell. Halving the node sizes was exactly that: a change no key could see.
/// A cache that cannot notice its own producer changed is the same defect as
/// a figure that was correct when computed and wrong when read.
const LAYOUT_VERSION: u32 = 4;

pub struct Cached {
    pub meta_json: String,
    pub data: Vec<u8>,
    pub from_cache: bool,
}

fn dir_for(cache_root: &Path, genesis: &str) -> PathBuf {
    cache_root.join(genesis)
}

fn paths(cache_root: &Path, genesis: &str, head: &str, view: View) -> (PathBuf, PathBuf) {
    let d = dir_for(cache_root, genesis);
    let v = view.as_str();
    (
        d.join(format!("layout-v{LAYOUT_VERSION}-{v}-{head}.json")),
        d.join(format!("layout-v{LAYOUT_VERSION}-{v}-{head}.bin")),
    )
}

pub fn load(cache_root: &Path, genesis: &str, head: &str, view: View) -> Option<Cached> {
    let (m, b) = paths(cache_root, genesis, head, view);
    let meta_json = std::fs::read_to_string(&m).ok()?;
    let data = std::fs::read(&b).ok()?;
    Some(Cached { meta_json, data, from_cache: true })
}

pub fn store(cache_root: &Path, genesis: &str, head: &str, l: &Layout) -> Result<String, String> {
    let view = l.meta.view;
    let d = dir_for(cache_root, genesis);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let meta_json = serde_json::to_string(&l.meta).map_err(|e| e.to_string())?;
    let (m, b) = paths(cache_root, genesis, head, view);
    std::fs::write(&m, &meta_json).map_err(|e| e.to_string())?;
    std::fs::write(&b, &l.data).map_err(|e| e.to_string())?;

    // A layout built from a HEAD that is no longer current is dead weight,
    // not history — the store it described has moved on. Sweep siblings.
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // Only this view's siblings: the other view built from the same
            // HEAD is still good.
            let this_view = format!("-{}-", view.as_str());
            if name.starts_with("layout-") && !name.contains(head) && name.contains(&this_view)
                || name.starts_with("layout-") && !name.starts_with(&format!("layout-v{LAYOUT_VERSION}-"))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(meta_json)
}

/// Ask the daemon for everything one soul's layout needs, in parallel, then
/// compute it.
pub async fn build_from_server(
    http: &reqwest::Client,
    daemon_port: u16,
    genesis: &str,
    head: &str,
    repo_path: &std::path::Path,
    view: View,
) -> Result<Layout, String> {
    let c = SparqlClient::for_soul(http.clone(), daemon_port, genesis);

    // Six reads, all through the one SPARQL endpoint. The old viewer's
    // `/api/viz/nodes` and `/api/viz/edges` were only ever SPARQL wearing a
    // REST hat; asking directly removes the dependency on that server and
    // lets the edge query be the one this view actually needs.
    let qs = layout::q_store_head();
    let store_head = c
        .query::<layout::StoreHeadRow>(&qs)
        .await
        .ok()
        .and_then(|v| v.into_iter().next())
        .map(|r| r.id);

    // How far behind, counted by git rather than guessed. Reported as
    // unknown when it cannot be determined — zero is the reassuring answer
    // and must never be the fallback.
    let commits_behind = match &store_head {
        Some(sh) if sh != head => tokio::process::Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("rev-list")
            .arg("--count")
            .arg(format!("{sh}..{head}"))
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()),
        Some(_) => Some(0),
        None => None,
    };

    if view == View::Base {
        return build_base(&c, genesis, head, repo_path, store_head, commits_behind).await;
    }

    // Ask for documents FIRST, alone, and stop if there are none.
    //
    // These seven queries used to fan out together and the empty-store check
    // ran after all of them had returned. On lUX — 1.5 million facts, and a
    // store whose `now` view an interrupted sync had never created — that
    // meant 26 seconds of work before reporting a problem the first query
    // already knew about, on every single load, with nothing cacheable at the
    // end of it. The cost of asking first is one extra round trip on the
    // healthy path; the saving is everything on the broken one.
    let nodes = c.query::<layout::NodeRow>(&layout::q_nodes()).await?;
    if nodes.is_empty() {
        // Two different situations, and only one of them is fixed by a sync.
        // A store that is current and still has no typed documents is a repo
        // nothing has typed — the base view is the answer there, not a sync.
        return Err(if store_head.as_deref() == Some(head) {
            "this store has no typed documents — nothing in this repo has been given a type"
                .to_string()
        } else {
            "this store has no documents in its `now` view — run `git lex sync` in the repo"
                .to_string()
        });
    }

    let (qe, qa, qb, qd, ql, qlb) = (
        layout::q_edges(),
        layout::q_alias(),
        layout::q_born(),
        layout::q_dates(),
        layout::q_labels(),
        layout::q_link_born(),
    );
    let (edges, aliases, born, dates, labels, link_born) = tokio::join!(
        c.query::<layout::EdgeRow>(&qe),
        c.query::<layout::AliasRow>(&qa),
        c.query::<layout::BornRow>(&qb),
        c.query::<layout::DateRow>(&qd),
        c.query::<layout::LabelRow>(&ql),
        c.query::<layout::LinkBornRow>(&qlb),
    );

    // The remaining reads may legitimately come back empty: a soul with no
    // links, or one whose history graph has not been built, still has a
    // legible spiral. What it must not do is pretend — `undated` and
    // `dropped` in the metadata carry the shortfall.
    Ok(layout::build(
        genesis,
        head,
        nodes,
        edges.unwrap_or_default(),
        link_born.unwrap_or_default(),
        aliases.unwrap_or_default(),
        born.unwrap_or_default(),
        dates.unwrap_or_default(),
        labels.unwrap_or_default(),
        store_head,
        commits_behind,
    ))
}

/// The base view: the store's file tree, dated by git.
async fn build_base(
    c: &SparqlClient,
    genesis: &str,
    head: &str,
    repo_path: &Path,
    store_head: Option<String>,
    commits_behind: Option<usize>,
) -> Result<Layout, String> {
    let Some(sh) = store_head.clone() else {
        return Err("this repo has no graph yet — run `git lex sync` in the repo".to_string());
    };
    let tree = c.query::<layout::PathRow>(&layout::q_filetree(&sh)).await?;
    if tree.is_empty() {
        return Err(format!(
            "the store has no file tree for commit {} — run `git lex sync` in the repo",
            &sh[..sh.len().min(8)]
        ));
    }
    if !tree.iter().any(|r| {
        let l = r.path.to_ascii_lowercase();
        (l.ends_with(".md") || l.ends_with(".markdown")) && !r.path.starts_with(".lex/")
    }) {
        return Err(format!(
            "this repo has no markdown documents — its {} tracked files are all code, data or git-lex's own",
            tree.len()
        ));
    }

    // History up to the commit the store was built from, not HEAD, so every
    // date in the picture describes the same moment as its file tree.
    let git = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["-c", "core.quotepath=off", "log", "--no-renames", "--format=%x00%H", "--name-only"])
        .arg(&sh)
        .output();

    let (qo, qe, qlb, qa, qd, ql) = (
        layout::q_ordinals(),
        layout::q_edges(),
        layout::q_link_born(),
        layout::q_alias(),
        layout::q_dates(),
        layout::q_labels(),
    );
    let (git, ords, edges, link_born, aliases, dates, labels) = tokio::join!(
        git,
        c.query::<layout::OrdinalRow>(&qo),
        c.query::<layout::EdgeRow>(&qe),
        c.query::<layout::LinkBornRow>(&qlb),
        c.query::<layout::AliasRow>(&qa),
        c.query::<layout::DateRow>(&qd),
        c.query::<layout::LabelRow>(&ql),
    );
    let git = git.map_err(|e| format!("could not run git: {e}"))?;
    if !git.status.success() {
        return Err(format!(
            "git could not read the history the store was built from: {}",
            String::from_utf8_lossy(&git.stderr).trim()
        ));
    }
    let history = layout::parse_git_log(&String::from_utf8_lossy(&git.stdout));
    // Without ordinals nothing can be dated, and every document would sit on
    // the rim looking new. That is a failure to report, not a picture.
    let ords = ords?;
    if ords.is_empty() {
        return Err("the store records no commits — run `git lex sync` in the repo".to_string());
    }

    Ok(layout::build_base(
        genesis,
        head,
        tree,
        &history,
        ords,
        edges.unwrap_or_default(),
        link_born.unwrap_or_default(),
        aliases.unwrap_or_default(),
        dates.unwrap_or_default(),
        labels.unwrap_or_default(),
        store_head,
        commits_behind,
    ))
}
