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

use crate::layout::{self, Layout};
use std::path::{Path, PathBuf};

pub struct Cached {
    pub meta_json: String,
    pub data: Vec<u8>,
    pub from_cache: bool,
}

fn dir_for(cache_root: &Path, genesis: &str) -> PathBuf {
    cache_root.join(genesis)
}

fn paths(cache_root: &Path, genesis: &str, head: &str) -> (PathBuf, PathBuf) {
    let d = dir_for(cache_root, genesis);
    (
        d.join(format!("layout-{head}.json")),
        d.join(format!("layout-{head}.bin")),
    )
}

pub fn load(cache_root: &Path, genesis: &str, head: &str) -> Option<Cached> {
    let (m, b) = paths(cache_root, genesis, head);
    let meta_json = std::fs::read_to_string(&m).ok()?;
    let data = std::fs::read(&b).ok()?;
    Some(Cached { meta_json, data, from_cache: true })
}

pub fn store(cache_root: &Path, genesis: &str, head: &str, l: &Layout) -> Result<String, String> {
    let d = dir_for(cache_root, genesis);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let meta_json = serde_json::to_string(&l.meta).map_err(|e| e.to_string())?;
    let (m, b) = paths(cache_root, genesis, head);
    std::fs::write(&m, &meta_json).map_err(|e| e.to_string())?;
    std::fs::write(&b, &l.data).map_err(|e| e.to_string())?;

    // A layout built from a HEAD that is no longer current is dead weight,
    // not history — the store it described has moved on. Sweep siblings.
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("layout-") && !name.contains(head) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(meta_json)
}

/// Ask one repo's server for everything the layout needs, in parallel, then
/// compute it.
pub async fn build_from_server(
    http: &reqwest::Client,
    port: u16,
    genesis: &str,
    head: &str,
) -> Result<Layout, String> {
    let base = format!("http://127.0.0.1:{port}");

    async fn rows<T: serde::de::DeserializeOwned>(
        http: &reqwest::Client,
        url: String,
    ) -> Result<Vec<T>, String> {
        let r = http.get(&url).send().await.map_err(|e| format!("{url}: {e}"))?;
        let v: serde_json::Value = r.json().await.map_err(|e| format!("{url}: {e}"))?;
        let arr = v.get("results").cloned().unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value(arr).map_err(|e| format!("{url}: {e}"))
    }

    async fn query<T: serde::de::DeserializeOwned>(
        http: &reqwest::Client,
        base: &str,
        q: String,
    ) -> Result<Vec<T>, String> {
        let r = http
            .post(format!("{base}/api/query"))
            .json(&serde_json::json!({ "query": q }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
        let arr = v.get("results").cloned().unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value(arr).map_err(|e| e.to_string())
    }

    let (nodes, edges, aliases, born, dates, labels) = tokio::join!(
        rows::<layout::NodeRow>(http, format!("{base}/api/viz/nodes")),
        rows::<layout::EdgeRow>(http, format!("{base}/api/viz/edges")),
        query::<layout::AliasRow>(http, &base, layout::q_alias()),
        query::<layout::BornRow>(http, &base, layout::q_born()),
        query::<layout::DateRow>(http, &base, layout::q_dates()),
        query::<layout::LabelRow>(http, &base, layout::q_labels()),
    );

    let nodes = nodes?;
    if nodes.is_empty() {
        return Err("this store has no documents — run `git lex sync` in the repo".to_string());
    }
    // The four remaining reads are allowed to come back empty rather than
    // fail the whole view: a soul with no edges, or one whose history graph
    // has not been built, still has a legible spiral. What it must not do is
    // pretend — `undated` and `dropped` in the metadata carry the shortfall.
    Ok(layout::build(
        genesis,
        head,
        nodes,
        edges.unwrap_or_default(),
        aliases.unwrap_or_default(),
        born.unwrap_or_default(),
        dates.unwrap_or_default(),
        labels.unwrap_or_default(),
    ))
}
