//! The whole-repo spiral layout, computed server-side.
//!
//! One view for every repo: every markdown file, dated by git, coloured by
//! its kit type where it has one and by its folder where it does not. There
//! used to be a second, typed-only view built from the store's `now` view;
//! it was removed on 2026-10-04 (goodlux), since the one view draws
//! everything it drew.
//!
//! A document's position is **when it first appeared**, drawn as a spiral:
//! centre is the first commit, rim is now, one turn is one slice of the
//! repo's life. Colour is class, size is how often it changed. Nothing
//! depends on the previous frame, so there is nothing to diverge and nothing
//! to settle — it draws the same picture every time, which is what makes it
//! something two people can talk about.
//!
//! It runs here rather than in the browser because it is a pure function of
//! commit ordinals: it changes only when HEAD changes, so it is computed
//! once, cached against the HEAD it was built from, and shipped as typed
//! arrays that go straight into GPU buffers.
//!
//! Four constraints are baked into the maths below, each paid for once.
//!
//! **Position is when a document was SAVED, not when it was written.**
//! Measured: median gap 2.9 minutes, but one document in six lands in the
//! wrong turn of the spiral and the worst case was 65 days. The view says
//! "saved" for that reason.
//!
//! **Import commits create false cohorts.** 51 documents share one import
//! commit here; 42 of 97 Things share a migration commit. They are spread
//! along the spiral's *normal*, never its angle — angle is time, and
//! spreading an import along the angle makes an afternoon read as three
//! weeks.
//!
//! **The File plane and the Thing plane are one join apart.** Each document
//! is folded onto its file through `gl:fileId` so a Thing and the File it
//! speaks through are one dot, not two.
//!
//! **Disclose what you drop.** Edges whose endpoints are not in the node set
//! are counted and reported, never silently discarded.

use serde::Serialize;
use std::collections::{HashMap, HashSet};

const GL_FILE: &str = "https://repolex.ai/ontology/git-lex/File";
const NOW: &str = "https://repolex.ai/git-lex/NamedGraph/now";
const ONE_GRAPH: &str = "https://repolex.ai/git-lex/LexHistoryGraph";
const COMMITS: &str = "https://repolex.ai/git-lex/NamedGraph/commits";

pub fn q_alias() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         SELECT ?thing ?file WHERE {{ GRAPH <{NOW}> {{ ?thing gl:fileId ?file }} }}"
    )
}

/// The kit type of every file that has one, read through its Thing.
///
/// A type lives on the Thing and the Thing names its file with `gl:fileId`,
/// which is complete on every soul measured (zero Things without a file,
/// 2026-09-23). So one join colours a file by what a kit says it is, and a
/// file no kit has typed simply does not come back — it keeps its folder.
pub fn q_types() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         SELECT ?file ?type WHERE {{
             GRAPH <{NOW}> {{ ?thing gl:fileId ?file ; a ?type FILTER(?type != <{GL_FILE}>) }}
         }}"
    )
}

/// The newest commit the STORE knows about.
///
/// This is not the repo's HEAD, and the gap between them is the single most
/// important thing this view can tell you. `git lex save` commits and
/// reconciles sidecars; it does NOT rebuild the store's `now` view. That
/// happens on `git lex sync`. So a soul can be four documents ahead of the
/// graph drawn from it, and every number on screen will be internally
/// consistent, correct as of some past moment, and wrong about today.
///
/// Found by committing four documents to my own soul and watching the picture
/// not change. A fresh endpoint saw the same 23 journals as the running one,
/// which is how I knew it was the store and not a server holding a snapshot.
/// It is also why lUX drew nothing for a day: its `now` view held zero
/// triples and nothing said so.
pub fn q_store_head() -> String {
    format!(
        "PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         SELECT ?id ?ord WHERE {{
             GRAPH <{COMMITS}> {{ ?c g2:ordinalDerived ?ord ; g2:id ?id }}
         }} ORDER BY DESC(xsd:integer(?ord)) LIMIT 1"
    )
}

/// Every link between documents.
///
/// Deliberately NOT the query the old viewer used. That one took `md:linksTo`
/// and then every predicate outside the `ontology/git-lex/` namespace — which
/// excludes `gl:relatedToId`, the predicate souls use to declare a reference
/// from one document to another. So the declared references were never drawn
/// at all: on lUX that is 14,529 links absent from a picture claiming to show
/// how the soul connects.
///
/// This asks the question the other way round. An edge is a triple from a
/// typed subject to an IRI that is not a class or the ontology itself, minus
/// three pieces of machinery: `rdf:type` is membership, `gl:fileId` is the
/// File/Thing join this layout already folds on, and `gl:id` is a node naming
/// itself. Dangling targets are deliberately kept — resolution happens in
/// `build`, where an unresolvable target is counted and disclosed rather than
/// filtered out here where nobody would ever see it.
/// When each link was first asserted, as a commit ordinal.
///
/// This is the real creation order, not a guess from the endpoints' ages.
/// git-lex reifies every statement and records the commit that asserted it,
/// so a link carries its own birthday: `?e rdf:reifies <<( ?s ?p ?o )>> ;
/// gl:assertedIn ?c`. Measured on W3BL0RD: 3,083 reified statements, of which
/// `md:linksTo` is the single most common at 731, spread across commits
/// 1-184. `gl:relatedToId` is reified too, 25 of them, all late (154-180) —
/// which is itself true history, since the declared-reference vocabulary only
/// arrived recently.
///
/// MIN because a statement is re-asserted every time its file is touched;
/// the first assertion is when the link came into being.
///
/// Worth recording how nearly this was not built: the first version of this
/// query named the wrong graph and returned zero rows, which reads exactly
/// like "the store does not have this" — the answer that would have had me
/// report it as not possible. The control (count reified statements at
/// all) is what separated a wrong query from a real absence. See
/// `feedback-a-label-is-a-claim-with-no-test`.
pub fn q_link_born() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
         SELECT ?from ?predicate ?target (MIN(?ord) AS ?born) WHERE {{
             GRAPH <{ONE_GRAPH}> {{
                 ?e rdf:reifies <<( ?s ?p ?o )>> ; gl:assertedIn ?c
             }}
             GRAPH <{COMMITS}> {{ ?c g2:ordinalDerived ?ord }}
             FILTER(isIRI(?o))
             FILTER(?s != ?o)
             FILTER(?p != rdf:type)
             FILTER(?p != gl:fileId)
             FILTER(?p != gl:id)
             FILTER(!STRSTARTS(STR(?o), \"https://repolex.ai/ontology/\"))
             BIND(STR(?s) AS ?from)
             BIND(STR(?p) AS ?predicate)
             BIND(STR(?o) AS ?target)
         }} GROUP BY ?from ?predicate ?target"
    )
}

pub fn q_edges() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         SELECT DISTINCT ?from ?predicate ?target WHERE {{
             GRAPH <{NOW}> {{
                 ?from ?p ?to .
                 ?from a ?tf .
                 FILTER(isIRI(?to))
                 FILTER(?from != ?to)
                 FILTER(?p != <http://www.w3.org/1999/02/22-rdf-syntax-ns#type>)
                 FILTER(?p != gl:fileId)
                 FILTER(?p != gl:id)
                 FILTER(!STRSTARTS(STR(?to), \"https://repolex.ai/ontology/\"))
                 BIND(STR(?p) AS ?predicate)
                 BIND(STR(?to) AS ?target)
             }}
         }}"
    )
}

/// Resolve every document's NAME, across both planes.
///
/// This is commit 2ebd895 carried forward, and it has to live here because
/// `/api/viz/nodes` returns each node's **slug**, not its title — so a view
/// built on that endpoint alone renders documents as identifiers, which is
/// exactly the behaviour that made souls show as hex strings for eight
/// months.
///
/// Four branches, because a title can be in four places. An unqualified
/// frontmatter key (`title:` rather than `soul.Note.title:`) cannot bind to a
/// class property, so git-lex emits it under the `fm/` fallback namespace
/// attached to the FILE subject — never to the Thing. Asking a Thing for
/// `fm:title` therefore matches nothing every time. Measured on W3BL0RD: 33
/// `fm:title` against 4 `gl:title`, and `gl:name` on exactly one subject in
/// the whole store, so roughly 89% of the titles that exist were unreachable
/// from the plane the query ran on.
///
/// A real Thing-plane name still wins where one exists; the file hop is a
/// fallback, not a replacement.
pub fn q_labels() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         SELECT ?s ?label ?rank WHERE {{
             GRAPH <{NOW}> {{
                 {{ ?s gl:name ?label . BIND(1 AS ?rank) }}
                 UNION
                 {{ ?s gl:title ?label . BIND(2 AS ?rank) }}
                 UNION
                 {{ ?s <https://repolex.ai/ontology/git-lex/fm/title> ?label . BIND(3 AS ?rank) }}
                 UNION
                 {{ ?s gl:fileId ?f . ?f <https://repolex.ai/ontology/git-lex/fm/title> ?label . BIND(4 AS ?rank) }}
             }}
         }}"
    )
}

/// Every file the store's tree lists at the commit it was synced from.
///
/// This is the one listing every synced repo has, typed or not. The `now`
/// view is built from frontmatter and links, so a repo whose markdown carries
/// neither can have no `now` view at all — git-lex itself is one — while its
/// file tree is complete.
pub fn q_filetree(store_head: &str) -> String {
    format!(
        "PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         SELECT ?path WHERE {{
             GRAPH <https://repolex.ai/git-lex/NamedGraph/filetree/{store_head}> {{ ?e g2:path ?path }}
         }}"
    )
}

/// Commit sha to the store's ordinal, so a date git gives us lands on the
/// same axis as the store's own link birthdays.
pub fn q_ordinals() -> String {
    format!(
        "PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         SELECT ?id ?ord WHERE {{ GRAPH <{COMMITS}> {{ ?c g2:id ?id ; g2:ordinalDerived ?ord }} }}"
    )
}

/// Two anchors and a direction is all an axis needs. Without dates the ticks
/// can say a turn closed but nothing about when.
pub fn q_dates() -> String {
    format!(
        "PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         SELECT ?ord ?when WHERE {{
             GRAPH <{COMMITS}> {{
                 ?c g2:ordinalDerived ?ord ; g2:author ?sig .
                 ?sig g2:xsdDateTimeDerived ?when .
             }}
         }}"
    )
}

/// A predicate that produced at least one drawn edge.
///
/// Carried because edge density is dominated by a handful of predicates and
/// the picture is unreadable without a way to switch them off: on lUX,
/// 14,529 of 18,131 drawn links are `relatedToId` alone. A legend you cannot
/// act on is decoration.
#[derive(Debug, Clone, Serialize)]
pub struct PredicateInfo {
    pub uri: String,
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassInfo {
    pub uri: String,
    pub name: String,
    pub count: usize,
    pub color: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocMeta {
    pub id: String,
    pub label: String,
    /// Index into `classes`.
    pub class: usize,
    /// Birth commit ordinal, or null when nothing dates it.
    pub born: Option<i64>,
    pub events: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dropped {
    pub reason: String,
    pub count: usize,
    pub examples: Vec<String>,
    /// Which predicates produced these, largest first.
    ///
    /// One total is nearly useless here. lUX drops 6,425 links whose target
    /// is not a document — and 5,365 of those are a single predicate,
    /// `copia:lookMomentId`, pointing at Moment records that were never
    /// documents in this store. That is a systematic fact about the shape of
    /// the data, not 6,425 pieces of rot, and the two call for completely
    /// different reactions. Grouping is the difference between a number and
    /// a finding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_predicate: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutMeta {
    pub genesis_sha: String,
    /// The repo's HEAD, read from git at request time.
    pub head_sha: String,
    /// The newest commit the store was synced from. When this differs from
    /// `head_sha` the picture is behind the repo and says so.
    pub store_head: Option<String>,
    /// How many commits the store is behind. `None` when it could not be
    /// determined — reported as unknown rather than as zero, since zero is
    /// the reassuring answer and must never be the guess.
    pub commits_behind: Option<usize>,
    pub built_at_ms: u128,
    pub node_count: usize,
    pub edge_count: usize,
    pub turns: u32,
    pub classes: Vec<ClassInfo>,
    /// Indexed by the `edge_predicates` array in the binary payload.
    pub predicates: Vec<PredicateInfo>,
    pub docs: Vec<DocMeta>,
    /// Ticks marking where each turn of the spiral closes, with the date the
    /// spiral had reached by then.
    pub turn_dates: Vec<Option<String>>,
    /// How many documents nothing could date. They sit on the rim, and they
    /// are counted here rather than being quietly placed as if they were new.
    pub undated: usize,
    /// Drawn links for which the store records no assertion commit. These
    /// cannot take part in a chronological replay, so the count is shown
    /// rather than folded into the timeline at ordinal 0.
    pub links_undated: usize,
    /// The commit ordinals the spiral actually spans — the birth commit of
    /// the oldest surviving document, and of the newest.
    ///
    /// These are NOT the repo's first and last commit, and the difference is
    /// not small: on lUX the oldest document still in the store was born at
    /// ordinal 104 of 3,487, so 103 commits of history precede the centre.
    /// The view used to caption the centre as "this soul's first commit",
    /// which was a claim about the repository made from data about its
    /// documents.
    pub first_ordinal: Option<i64>,
    pub last_ordinal: Option<i64>,
    /// Files in the tree that are not markdown, and so are
    /// not drawn. Counted so a code repo does not read as a tiny one.
    pub other_files: usize,
    /// Markdown under `.lex/`, which is git-lex's own
    /// machinery (kit copies, the compact ontology) rather than anybody's
    /// writing. Left out of the picture, and counted.
    pub machinery_files: usize,
    /// How many documents got a real title rather than falling back to their
    /// slug. Printed, because "titles are resolved" is a claim and this is
    /// its test.
    pub titled: usize,
    pub dropped: Vec<Dropped>,
    /// Byte offsets into the companion binary payload.
    pub offsets: Offsets,
}

#[derive(Debug, Clone, Serialize)]
pub struct Offsets {
    /// f32 x2 per node.
    pub positions: usize,
    pub positions_bytes: usize,
    /// u8 x3 per node.
    pub colors: usize,
    pub colors_bytes: usize,
    /// f32 per node.
    pub sizes: usize,
    pub sizes_bytes: usize,
    /// u32 x2 per edge.
    pub edges: usize,
    pub edges_bytes: usize,
    /// u16 per edge: an index into `predicates`.
    pub edge_predicates: usize,
    pub edge_predicates_bytes: usize,
    /// Commit ordinal each link was first asserted in, u32 per edge, parallel
    /// to `edges`. `u32::MAX` means the store records no birthday for it.
    pub edge_born: usize,
    pub edge_born_bytes: usize,
    pub total: usize,
}

pub struct Layout {
    pub meta: LayoutMeta,
    pub data: Vec<u8>,
}

/// Rows as the child server returns them.
#[derive(serde::Deserialize)]
pub struct EdgeRow {
    pub from: String,
    pub target: String,
    #[serde(default)]
    pub predicate: Option<String>,
}

/// One link and the commit ordinal it was first asserted in.
#[derive(serde::Deserialize)]
pub struct LinkBornRow {
    pub from: String,
    pub predicate: String,
    pub target: String,
    pub born: String,
}

#[derive(serde::Deserialize)]
pub struct AliasRow {
    pub thing: String,
    pub file: String,
}

#[derive(serde::Deserialize)]
pub struct TypeRow {
    pub file: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(serde::Deserialize)]
pub struct LabelRow {
    pub s: String,
    pub label: String,
    pub rank: String,
}

#[derive(serde::Deserialize)]
pub struct StoreHeadRow {
    pub id: String,
    pub ord: String,
}

#[derive(serde::Deserialize)]
pub struct PathRow {
    pub path: String,
}

#[derive(serde::Deserialize)]
pub struct OrdinalRow {
    pub id: String,
    pub ord: String,
}

/// What git says about one path: the oldest commit it appears in, and how
/// many commits touched it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathHistory {
    pub first_sha: String,
    pub commits: u32,
}

/// Read `git log --no-renames --format=%x00%H --name-only` output.
///
/// The log runs newest first, so the last commit seen for a path is the one
/// it first appeared in. `--no-renames` is deliberate: a renamed file counts
/// as appearing at its new path in the commit that moved it, which is when a
/// document at that path began to exist. Following renames would date it to
/// a path the picture does not show.
pub fn parse_git_log(out: &str) -> HashMap<String, PathHistory> {
    let mut map: HashMap<String, PathHistory> = HashMap::new();
    for block in out.split('\0').skip(1) {
        let mut lines = block.lines();
        let Some(sha) = lines.next().map(str::trim) else { continue };
        for path in lines.map(str::trim).filter(|l| !l.is_empty()) {
            let e = map.entry(path.to_string()).or_default();
            e.first_sha = sha.to_string();
            e.commits += 1;
        }
    }
    map
}

#[derive(serde::Deserialize)]
pub struct DateRow {
    pub ord: String,
    pub when: String,
}

fn short_name(uri: &str) -> String {
    uri.rsplit(['/', '#']).next().unwrap_or(uri).to_string()
}

/// Turns scale with the size of the soul, not fixed. Seven turns of a
/// 6,000-document soul is a legible year-by-year spiral; seven turns of a
/// 130-document one is confetti — too few dots per turn for an arc to read
/// as an arc. Roughly 900 documents per turn, and a young soul gets a ring.
fn turns_for(n: usize) -> u32 {
    (((n as f64) / 900.0).ceil() as u32).clamp(2, 7)
}

/// Scale dots up on a sparse spiral, and never down on a crowded one.
///
/// Dot size was tuned by @goodlux on 2026-09-04 against a soul of a couple of
/// hundred documents, where the complaint was that dots read as blobs. The
/// number that came out of that is right for that density and only that
/// density: the spiral fills the view whatever it holds, so a repo with a
/// fifth of the documents gets a fifth of the neighbours, and the same dots
/// read as dust. Measured on a 21-document corpus, 2026-09-23 — the size of
/// corpus someone brings to git-lex on their first day.
///
/// `REFERENCE` is documents-per-turn at the density that tuning was done at,
/// so the scale is exactly 1.0 there and for everything denser: no existing
/// soul changes. Sparse repos open up, by at most `MAX`, which stops a
/// five-document repo drawing five balloons.
fn density_scale(n: usize, turns: u32) -> f32 {
    const REFERENCE: f32 = 90.0;
    const MAX: f32 = 2.5;
    if n == 0 || turns == 0 {
        return 1.0;
    }
    (REFERENCE / (n as f32 / turns as f32)).sqrt().clamp(1.0, MAX)
}

/// A readable spread of hues. Strided rather than walked, because
/// consecutive entries landed on near-identical blues and a legend you
/// cannot read is a legend that lies.
fn color_for(i: usize) -> String {
    const PALETTE: [&str; 12] = [
        "#1f77b4", "#d62728", "#2ca02c", "#9467bd", "#ff7f0e", "#8c564b",
        "#17becf", "#e377c2", "#7f7f7f", "#bcbd22", "#393b79", "#a55194",
    ];
    PALETTE[(i * 5) % PALETTE.len()].to_string()
}

fn hex_rgb(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let p = |a: usize| u8::from_str_radix(&h[a..a + 2], 16).unwrap_or(136);
    if h.len() >= 6 { [p(0), p(2), p(4)] } else { [136, 136, 136] }
}

/// Deterministic hash — the same document gets the same speck every session.
fn hash01(s: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    (h as f64 / u32::MAX as f64) as f32
}

pub const FILE_PREFIX: &str = "https://repolex.ai/git-lex/File/";

fn is_markdown(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".md") || l.ends_with(".markdown")
}

/// A folder is split one level further when it holds more than this share of
/// the documents. lUX keeps 94% of its markdown under `Copia/`; coloured by
/// top folder that is a one-colour picture that says nothing. A third, not a
/// majority: at 60% W3BL0RD (51% `Soul/`) still drew every journal, note and
/// exploration in one colour, which is the one distinction a soul's owner
/// most wants to see. At most two folders can pass a third, so the legend
/// stays short.
const SPLIT_SHARE: f64 = 1.0 / 3.0;
/// Types and folders past this many, together, get a shared grey entry per
/// kind, so the palette is never reused for two different entries.
const MAX_NAMED: usize = 11;

fn top(path: &str) -> &str {
    match path.find('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

fn two_deep(path: &str) -> &str {
    let Some(a) = path.find('/') else { return "" };
    match path[a + 1..].find('/') {
        Some(b) => &path[..a + 1 + b],
        None => &path[..a],
    }
}

/// The base view: every markdown file, by folder, dated by git.
#[allow(clippy::too_many_arguments)]
pub fn build_base(
    genesis_sha: &str,
    head_sha: &str,
    tree: Vec<PathRow>,
    history: &HashMap<String, PathHistory>,
    ordinals: Vec<OrdinalRow>,
    edges: Vec<EdgeRow>,
    link_born_rows: Vec<LinkBornRow>,
    aliases: Vec<AliasRow>,
    type_rows: Vec<TypeRow>,
    date_rows: Vec<DateRow>,
    label_rows: Vec<LabelRow>,
    store_head: Option<String>,
    commits_behind: Option<usize>,
) -> Layout {
    let ord_of: HashMap<&str, i64> = ordinals
        .iter()
        .filter_map(|r| r.ord.parse().ok().map(|o| (r.id.as_str(), o)))
        .collect();

    let mut paths: Vec<&str> = Vec::new();
    let mut other_files = 0usize;
    let mut machinery_files = 0usize;
    let mut seen: HashSet<&str> = HashSet::new();
    for r in &tree {
        if !seen.insert(r.path.as_str()) {
            continue;
        }
        if !is_markdown(&r.path) {
            other_files += 1;
        } else if r.path.starts_with(".lex/") {
            machinery_files += 1;
        } else {
            paths.push(r.path.as_str());
        }
    }

    // Titles, where a document declares one. The base view reads them but
    // never needs them: a file name is always there.
    let file_of_thing: HashMap<&str, &str> =
        aliases.iter().map(|a| (a.thing.as_str(), a.file.as_str())).collect();
    let fold = |id: &str| -> String {
        file_of_thing.get(id).map(|s| s.to_string()).unwrap_or_else(|| id.to_string())
    };
    let mut best_label: HashMap<String, (u8, String)> = HashMap::new();
    for r in &label_rows {
        let rank: u8 = r.rank.parse().unwrap_or(9);
        let k = fold(&r.s);
        let e = best_label.entry(k).or_insert((rank, r.label.clone()));
        if rank < e.0 {
            *e = (rank, r.label.clone());
        }
    }

    // --- what colours each file ---------------------------------------------
    //
    // One view for every repo (goodlux, 2026-09-24). A file a kit has typed
    // is coloured by its type; a file nothing has typed is coloured by its
    // folder. A plain repo therefore draws exactly as the old base view did,
    // and a soul draws its Notes and Journals as types with only the untyped
    // remainder (Harness/, say) falling back to folders. Before this there
    // were two views and you had to know which one had the picture in it.
    //
    // Where a file carries more than one type, the first in IRI order is
    // taken, so the choice is the same on every load.
    let mut type_of: HashMap<String, &str> = HashMap::new();
    for r in &type_rows {
        // File IRIs carry the path unencoded, the same form the ids below
        // are built in (checked: no `%` in any W3BL0RD file IRI).
        let rel = r.file.strip_prefix(FILE_PREFIX).unwrap_or(&r.file);
        let e = type_of.entry(rel.to_string()).or_insert(r.ty.as_str());
        if r.ty.as_str() < *e {
            *e = r.ty.as_str();
        }
    }
    let untyped: Vec<&str> = paths.iter().copied().filter(|p| !type_of.contains_key(*p)).collect();

    // Folder for each untyped path: the top folder, unless that folder is
    // most of what is left, in which case its subfolders. Measured over the
    // untyped files only — on a soul, `Soul/` is typed away and `Harness/` is
    // what remains to be told apart.
    let mut top_counts: HashMap<&str, usize> = HashMap::new();
    for p in &untyped {
        *top_counts.entry(top(p)).or_insert(0) += 1;
    }
    let split: HashSet<&str> = top_counts
        .iter()
        .filter(|(k, c)| !k.is_empty() && **c as f64 > untyped.len() as f64 * SPLIT_SHARE)
        .map(|(k, _)| *k)
        .collect();
    let folder_of = |p: &str| -> String {
        let t = top(p);
        if split.contains(t) { two_deep(p).to_string() } else { t.to_string() }
    };

    // Types and folders share one palette. Types come first because a type is
    // something someone declared and a folder is only where a file sits.
    // Past MAX_NAMED the tail shares a grey entry per kind, so no two entries
    // ever quietly share a hue.
    let mut type_counts: HashMap<&str, usize> = HashMap::new();
    for t in type_of.values() {
        *type_counts.entry(*t).or_insert(0) += 1;
    }
    let mut folder_counts: HashMap<String, usize> = HashMap::new();
    for p in &untyped {
        *folder_counts.entry(folder_of(p)).or_insert(0) += 1;
    }
    let mut types_ranked: Vec<(String, usize)> =
        type_counts.into_iter().map(|(k, c)| (k.to_string(), c)).collect();
    types_ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut folders_ranked: Vec<(String, usize)> =
        folder_counts.into_iter().filter(|(f, _)| !f.is_empty()).collect();
    folders_ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let named_types: HashSet<String> =
        types_ranked.iter().take(MAX_NAMED).map(|(k, _)| k.clone()).collect();
    let named_folders: HashSet<String> = folders_ranked
        .iter()
        .take(MAX_NAMED.saturating_sub(named_types.len()))
        .map(|(k, _)| k.clone())
        .collect();

    const ROOT: &str = "(top level)";
    const REST: &str = "(other folders)";
    const REST_TYPES: &str = "(other types)";
    let legend_key = |p: &str| -> String {
        if let Some(t) = type_of.get(p) {
            return if named_types.contains(*t) { t.to_string() } else { REST_TYPES.to_string() };
        }
        let f = folder_of(p);
        if f.is_empty() {
            ROOT.to_string()
        } else if named_folders.contains(&f) {
            f
        } else {
            REST.to_string()
        }
    };

    let docs: Vec<Placed> = paths
        .iter()
        .map(|p| {
            let id = format!("{FILE_PREFIX}{p}");
            let h = history.get(*p);
            let label = best_label.get(&id).map(|(_, l)| l.clone());
            Placed {
                titled: label.is_some(),
                label: label.unwrap_or_else(|| short_name(p)),
                group: legend_key(p),
                born: h.and_then(|h| ord_of.get(h.first_sha.as_str()).copied()),
                events: h.map_or(0, |h| h.commits),
                id,
            }
        })
        .collect();

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for d in &docs {
        *counts.entry(d.group.as_str()).or_insert(0) += 1;
    }
    let mut order: Vec<(&str, usize)> = counts.into_iter().collect();
    // Types first, then real folders, each by size; the top level and the
    // two catch-alls go last, in grey, because none of them is a place or a
    // kind anyone chose.
    let rank = |k: &str| match k {
        _ if named_types.contains(k) => 0,
        ROOT => 2,
        REST_TYPES => 3,
        REST => 4,
        _ => 1,
    };
    order.sort_by(|a, b| rank(a.0).cmp(&rank(b.0)).then(b.1.cmp(&a.1)).then(a.0.cmp(b.0)));
    let classes: Vec<ClassInfo> = order
        .iter()
        .enumerate()
        .map(|(i, (k, c))| ClassInfo {
            uri: k.to_string(),
            name: match rank(k) {
                0 => short_name(k),
                1 => format!("{k}/"),
                _ => k.to_string(),
            },
            count: *c,
            color: match rank(k) {
                0 | 1 => color_for(i),
                2 => "#8d8d93".to_string(),
                _ => "#c4c4c9".to_string(),
            },
        })
        .collect();

    place_and_pack(
        Shared { genesis_sha, head_sha, store_head, commits_behind },
        docs,
        classes,
        &fold,
        edges,
        link_born_rows,
        date_rows,
        Census {
            other_files,
            machinery_files,
        },
    )
}

/// One document, ready to be placed. Its group is a kit type or a folder,
/// and nothing below this point knows which.
struct Placed {
    id: String,
    label: String,
    /// The legend entry this document belongs to — a class IRI or a folder.
    group: String,
    /// Whether `label` is a real title rather than the file name.
    titled: bool,
    born: Option<i64>,
    events: u32,
}

struct Shared<'a> {
    genesis_sha: &'a str,
    head_sha: &'a str,
    store_head: Option<String>,
    commits_behind: Option<usize>,
}

/// Counts of what was not drawn, carried through to the metadata unchanged.
struct Census {
    other_files: usize,
    machinery_files: usize,
}

/// Place documents on the spiral, resolve links, and pack the result.
///
/// Everything here is the same for every document: angle is the birth commit,
/// size is how often a document changed, colour is its legend entry.
#[allow(clippy::too_many_arguments)]
fn place_and_pack(
    shared: Shared<'_>,
    mut docs: Vec<Placed>,
    classes: Vec<ClassInfo>,
    fold: &dyn Fn(&str) -> String,
    edges: Vec<EdgeRow>,
    link_born_rows: Vec<LinkBornRow>,
    date_rows: Vec<DateRow>,
    census: Census,
) -> Layout {
    // Stable order, so the same repo packs identically twice running.
    docs.sort_by(|a, b| a.id.cmp(&b.id));
    let class_index: HashMap<&str, usize> =
        classes.iter().enumerate().map(|(i, c)| (c.uri.as_str(), i)).collect();

    // --- birth groups -----------------------------------------------------
    // Position within the cohort of documents born in the same commit.
    let mut cohorts: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, d) in docs.iter().enumerate() {
        cohorts.entry(d.born.unwrap_or(i64::MIN)).or_default().push(i);
    }
    let mut cohort_idx = vec![0usize; docs.len()];
    let mut cohort_size = vec![1usize; docs.len()];
    for g in cohorts.values() {
        for (k, &i) in g.iter().enumerate() {
            cohort_idx[i] = k;
            cohort_size[i] = g.len();
        }
    }

    let dated: Vec<i64> = docs.iter().filter_map(|d| d.born).collect();
    let min_b = dated.iter().copied().min().unwrap_or(0);
    let max_b = dated.iter().copied().max().unwrap_or(1);
    let span = (max_b - min_b).max(1) as f32;

    let n = docs.len();
    let turns = turns_for(n);
    let dscale = density_scale(n, turns);
    let mut positions = vec![0f32; n * 2];
    let mut colors = vec![0u8; n * 3];
    let mut sizes = vec![0f32; n];
    let mut undated = 0usize;

    for (i, d) in docs.iter().enumerate() {
        let t = match d.born {
            Some(b) => (b - min_b) as f32 / span,
            None => {
                undated += 1;
                1.0
            }
        };
        let theta = 2.0 * std::f32::consts::PI * turns as f32 * t;
        let r = 0.12 + 0.86 * t;

        // Documents born in the same commit are spread ACROSS the track,
        // never along it. Spreading along the angle made a 51-document
        // import cover 29 degrees of arc and read as a stretch of work —
        // the exact misreading this spread exists to prevent, reappearing
        // one level down. Time is the only thing allowed to move a dot
        // around the spiral; how many there were moves it across.
        let dr = 0.86f32;
        let dth = 2.0 * std::f32::consts::PI * turns as f32;
        let tx = dr * theta.cos() - r * dth * theta.sin();
        let ty = dr * theta.sin() + r * dth * theta.cos();
        let tl = tx.hypot(ty).max(f32::EPSILON);
        let (nx, ny) = (-ty / tl, tx / tl);

        let turn_gap = 0.86 / turns as f32;
        let spread = (turn_gap * 0.55)
            .min(0.012 * ((cohort_size[i] as f32 + 1.0).log2()).max(1.0));
        let across = if cohort_size[i] > 1 {
            ((cohort_idx[i] as f32 + 0.5) / cohort_size[i] as f32 - 0.5) * 2.0 * spread
        } else {
            0.0
        };
        // A whisper of deterministic scatter, also across the track, so that
        // equal-sized groups do not render as a ruler.
        let wobble = (hash01(&d.id) - 0.5) * (turn_gap * 0.12).min(0.02);

        positions[i * 2] = theta.cos() * r + nx * (across + wobble);
        positions[i * 2 + 1] = theta.sin() * r + ny * (across + wobble);

        let ci = class_index.get(d.group.as_str()).copied().unwrap_or(0);
        let rgb = hex_rgb(&classes[ci].color);
        colors[i * 3] = rgb[0];
        colors[i * 3 + 1] = rgb[1];
        colors[i * 3 + 2] = rgb[2];

        // Size is how many times the document changed, compressed so a
        // 300-event document does not swallow its neighbours, then opened up
        // by however much empty track this repo has. See `density_scale`:
        // the 2026-09-04 tuning was done on a dense soul and quietly assumed
        // every repo would be dense. See also LAYOUT_VERSION in
        // layout_api.rs — the cache is keyed by HEAD, which cannot see a
        // change to this line.
        sizes[i] = 1.6 * dscale * (1.0 + (d.events as f32 + 1.0).log2() * 0.28);
    }

    // --- edges ------------------------------------------------------------
    let index_of: HashMap<&str, u32> =
        docs.iter().enumerate().map(|(i, d)| (d.id.as_str(), i as u32)).collect();
    let mut edge_pairs: Vec<u32> = Vec::with_capacity(edges.len() * 2);
    let mut edge_preds: Vec<String> = Vec::with_capacity(edges.len());
    // When each link was first asserted, keyed the same way the edge is —
    // AFTER folding the Thing plane onto the File plane, or a Thing-plane
    // link would never match the folded edge it produced.
    let link_born: HashMap<(String, String, String), u32> = link_born_rows
        .iter()
        .filter_map(|r| {
            let ord: u32 = r.born.parse().ok()?;
            Some(((fold(&r.from), r.predicate.clone(), fold(&r.target)), ord))
        })
        .collect();
    let mut edge_born: Vec<u32> = Vec::with_capacity(edges.len());
    // Links whose birthday the store does not record. Counted and reported,
    // never quietly given ordinal 0 — 0 is "at the very beginning", which is
    // a confident claim about history and the wrong one.
    let mut links_undated = 0usize;
    let mut missing_target: Vec<String> = Vec::new();
    let mut missing_source: Vec<String> = Vec::new();
    let mut target_by_pred: HashMap<String, usize> = HashMap::new();
    let mut source_by_pred: HashMap<String, usize> = HashMap::new();
    let mut self_edges = 0usize;
    let mut dropped_targets = 0usize;

    for e in &edges {
        let a = fold(&e.from);
        let b = fold(&e.target);
        let pred = e.predicate.clone().unwrap_or_else(|| "(no predicate)".to_string());
        match (index_of.get(a.as_str()), index_of.get(b.as_str())) {
            (Some(&ai), Some(&bi)) => {
                if ai == bi {
                    self_edges += 1;
                    continue;
                }
                edge_pairs.push(ai);
                edge_pairs.push(bi);
                let key = (a.clone(), pred.clone(), b.clone());
                match link_born.get(&key) {
                    Some(&o) => edge_born.push(o),
                    None => {
                        links_undated += 1;
                        edge_born.push(u32::MAX);
                    }
                }
                edge_preds.push(pred);
            }
            (Some(_), None) => {
                dropped_targets += 1;
                *target_by_pred.entry(pred).or_insert(0) += 1;
                if missing_target.len() < 8 {
                    missing_target.push(e.target.clone());
                }
            }
            (None, _) => {
                *source_by_pred.entry(pred).or_insert(0) += 1;
                if missing_source.len() < 8 {
                    missing_source.push(e.from.clone());
                }
            }
        }
    }

    // Predicate table, largest first, so the legend reads as a census and
    // the index is stable for a given soul.
    let mut pred_counts: HashMap<&str, usize> = HashMap::new();
    for p in &edge_preds {
        *pred_counts.entry(p.as_str()).or_insert(0) += 1;
    }
    let mut pred_ranked: Vec<(&str, usize)> = pred_counts.into_iter().collect();
    pred_ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let predicates: Vec<PredicateInfo> = pred_ranked
        .iter()
        .map(|(uri, count)| PredicateInfo {
            uri: uri.to_string(),
            name: short_name(uri),
            count: *count,
        })
        .collect();
    let pred_index: HashMap<&str, u16> = predicates
        .iter()
        .enumerate()
        .map(|(i, p)| (p.uri.as_str(), i as u16))
        .collect();
    let edge_pred_idx: Vec<u16> = edge_preds
        .iter()
        .map(|p| pred_index.get(p.as_str()).copied().unwrap_or(0))
        .collect();

    fn rank_by_count(m: HashMap<String, usize>) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
    let missing_source_total: usize = source_by_pred.values().sum();

    let mut dropped = Vec::new();
    if dropped_targets > 0 {
        dropped.push(Dropped {
            // A dangling reference to a deleted document is history, not an
            // error — often the only surviving evidence the target existed.
            reason: "the link points at something that is not a document in this store"
                .to_string(),
            count: dropped_targets,
            examples: missing_target,
            by_predicate: rank_by_count(target_by_pred),
        });
    }
    if missing_source_total > 0 {
        dropped.push(Dropped {
            reason: "the document the link is written in is not itself in the node set".to_string(),
            count: missing_source_total,
            examples: missing_source,
            by_predicate: rank_by_count(source_by_pred),
        });
    }
    if self_edges > 0 {
        dropped.push(Dropped {
            reason: "the link points at the document it is written in".to_string(),
            count: self_edges,
            examples: vec![],
            by_predicate: vec![],
        });
    }

    // --- turn dates -------------------------------------------------------
    let mut ord_dates: Vec<(i64, String)> = date_rows
        .iter()
        .filter_map(|d| d.ord.parse::<i64>().ok().map(|o| (o, d.when.clone())))
        .collect();
    ord_dates.sort_by_key(|(o, _)| *o);
    let date_at = |ord: i64| -> Option<String> {
        match ord_dates.binary_search_by_key(&ord, |(o, _)| *o) {
            Ok(i) => Some(ord_dates[i].1.clone()),
            Err(i) if i < ord_dates.len() => Some(ord_dates[i].1.clone()),
            Err(_) => ord_dates.last().map(|(_, w)| w.clone()),
        }
    };
    let turn_dates: Vec<Option<String>> = (0..=turns)
        .map(|k| {
            let t = k as f32 / turns as f32;
            date_at(min_b + (t * span) as i64)
        })
        .collect();

    // --- pack -------------------------------------------------------------
    // Positions and colours go straight into GPU buffers, so they travel as
    // typed arrays rather than as JSON numbers. Every offset is aligned to 4
    // bytes so the browser can wrap them without copying.
    let align4 = |x: usize| (x + 3) & !3;
    let pos_bytes = positions.len() * 4;
    let col_bytes = colors.len();
    let siz_bytes = sizes.len() * 4;
    let edg_bytes = edge_pairs.len() * 4;
    let ep_bytes = edge_pred_idx.len() * 2;
    let eb_bytes = edge_born.len() * 4;

    let pos_at = 0;
    let col_at = align4(pos_at + pos_bytes);
    let siz_at = align4(col_at + col_bytes);
    let edg_at = align4(siz_at + siz_bytes);
    let ep_at = align4(edg_at + edg_bytes);
    let eb_at = align4(ep_at + ep_bytes);
    let total = eb_at + eb_bytes;

    let mut data = vec![0u8; total];
    for (i, v) in positions.iter().enumerate() {
        data[pos_at + i * 4..pos_at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    data[col_at..col_at + col_bytes].copy_from_slice(&colors);
    for (i, v) in sizes.iter().enumerate() {
        data[siz_at + i * 4..siz_at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in edge_pairs.iter().enumerate() {
        data[edg_at + i * 4..edg_at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in edge_pred_idx.iter().enumerate() {
        data[ep_at + i * 2..ep_at + i * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in edge_born.iter().enumerate() {
        data[eb_at + i * 4..eb_at + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }

    let titled = docs.iter().filter(|d| d.titled).count();

    let doc_meta: Vec<DocMeta> = docs
        .iter()
        .map(|d| DocMeta {
            id: d.id.clone(),
            label: d.label.clone(),
            class: class_index.get(d.group.as_str()).copied().unwrap_or(0),
            born: d.born,
            events: d.events,
        })
        .collect();

    Layout {
        meta: LayoutMeta {
            genesis_sha: shared.genesis_sha.to_string(),
            head_sha: shared.head_sha.to_string(),
            store_head: shared.store_head,
            commits_behind: shared.commits_behind,
            built_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            node_count: n,
            edge_count: edge_pairs.len() / 2,
            turns,
            classes,
            predicates,
            docs: doc_meta,
            turn_dates,
            undated,
            links_undated,
            first_ordinal: if dated.is_empty() { None } else { Some(min_b) },
            last_ordinal: if dated.is_empty() { None } else { Some(max_b) },
            other_files: census.other_files,
            machinery_files: census.machinery_files,
            titled,
            dropped,
            offsets: Offsets {
                positions: pos_at,
                positions_bytes: pos_bytes,
                colors: col_at,
                colors_bytes: col_bytes,
                sizes: siz_at,
                sizes_bytes: siz_bytes,
                edges: edg_at,
                edges_bytes: edg_bytes,
                edge_predicates: ep_at,
                edge_predicates_bytes: ep_bytes,
                edge_born: eb_at,
                edge_born_bytes: eb_bytes,
                total,
            },
        },
        data,
    }
}

#[cfg(test)]
mod tests {
    /// The tuning @goodlux did on a dense soul must survive untouched. A repo
    /// at or above that density gets exactly the size it got before.
    #[test]
    fn a_crowded_spiral_keeps_the_size_it_was_tuned_to() {
        assert_eq!(super::density_scale(180, 2), 1.0);
        assert_eq!(super::density_scale(13_000, 7), 1.0);
    }

    /// The case this exists for: a first-day corpus, where the same dots on
    /// the same spiral have a fraction of the neighbours and read as dust.
    #[test]
    fn a_sparse_spiral_opens_its_dots_up() {
        let s = super::density_scale(21, 2);
        assert!(s > 1.0, "a 21-document repo must not draw at dense-soul size");
        assert!(s <= 2.5, "and must not draw balloons either, got {s}");
    }

    /// Bounded at both ends. A near-empty repo is the easiest way to get a
    /// divide-by-something-tiny and a screenful of circles.
    #[test]
    fn the_scale_is_bounded_however_empty_the_repo_is() {
        for n in [0usize, 1, 2, 5] {
            let s = super::density_scale(n, 2);
            assert!((1.0..=2.5).contains(&s), "n={n} gave {s}");
        }
        assert_eq!(super::density_scale(10, 0), 1.0, "turns of zero must not divide");
    }

    use super::*;

    /// Every document must land within the spiral's own track, offset only
    /// along the normal. Spreading a same-commit cohort along the ANGLE would
    /// make an afternoon's import read as weeks of work, and the failure is
    /// invisible in a screenshot — it just looks like a busier soul.
    #[test]
    fn same_commit_documents_spread_across_the_track_not_along_it() {
        // 40 documents, all born in one commit, plus one earlier and one later
        // so the spiral has a span to place them on.
        let mut paths: Vec<String> = (0..40).map(|i| format!("doc{i}.md")).collect();
        paths.extend(["first.md".to_string(), "last.md".to_string()]);
        let mut history = HashMap::new();
        for p in &paths {
            let sha = match p.as_str() {
                "first.md" => "c1",
                "last.md" => "c100",
                _ => "c50",
            };
            history.insert(p.clone(), PathHistory { first_sha: sha.into(), commits: 1 });
        }
        let ords = [1, 50, 100]
            .iter()
            .map(|o| OrdinalRow { id: format!("c{o}"), ord: o.to_string() })
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let l = build_base(
            "g", "h", tree(&refs), &history, ords,
            vec![], vec![], vec![], vec![], vec![], vec![], None, None,
        );
        let turns = l.meta.turns as f32;

        // Recover each cohort member's angle and radius.
        let mut angles = vec![];
        for (i, d) in l.meta.docs.iter().enumerate() {
            if d.born != Some(50) {
                continue;
            }
            let at = l.meta.offsets.positions + i * 8;
            let x = f32::from_le_bytes(l.data[at..at + 4].try_into().unwrap());
            let y = f32::from_le_bytes(l.data[at + 4..at + 8].try_into().unwrap());
            angles.push(y.atan2(x));
            // Distance from the ideal track point for this document's birth.
            let t = (50.0 - 1.0) / 99.0;
            let th = 2.0 * std::f32::consts::PI * turns * t;
            let r = 0.12 + 0.86 * t;
            let d = ((x - th.cos() * r).powi(2) + (y - th.sin() * r).powi(2)).sqrt();
            let turn_gap = 0.86 / turns;
            assert!(
                d <= turn_gap * 0.55 + 0.02,
                "document sits {d} from its birth point on the track — further than the normal spread allows"
            );
        }
        assert_eq!(angles.len(), 40);

        // The whole cohort must occupy a narrow wedge. Spread along the angle
        // it would fan out; spread along the normal it barely moves.
        let (lo, hi) = angles.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        let wedge_degrees = (hi - lo).to_degrees();
        assert!(
            wedge_degrees < 12.0,
            "a 40-document import spans {wedge_degrees:.1} degrees of arc — angle is time, so that reads as a stretch of work that never happened"
        );
    }

    /// git lists newest first, so a path's birth is the LAST commit it
    /// appears under, and every appearance counts as a change.
    #[test]
    fn a_path_is_born_in_the_oldest_commit_that_lists_it() {
        let log = "\0ccc\n\na.md\nb.md\n\0bbb\n\na.md\n\0aaa\n\na.md\n";
        let h = parse_git_log(log);
        assert_eq!(h["a.md"], PathHistory { first_sha: "aaa".into(), commits: 3 });
        assert_eq!(h["b.md"], PathHistory { first_sha: "ccc".into(), commits: 1 });
        assert_eq!(h.len(), 2);
    }

    fn tree(paths: &[&str]) -> Vec<PathRow> {
        paths.iter().map(|p| PathRow { path: p.to_string() }).collect()
    }

    fn base(paths: &[&str]) -> Layout {
        let mut history = HashMap::new();
        for (i, p) in paths.iter().enumerate() {
            history.insert(p.to_string(), PathHistory { first_sha: format!("c{i}"), commits: 1 });
        }
        let ords = (0..paths.len())
            .map(|i| OrdinalRow { id: format!("c{i}"), ord: (i + 1).to_string() })
            .collect();
        build_base("g", "h", tree(paths), &history, ords, vec![], vec![], vec![], vec![], vec![], vec![], None, None)
    }

    /// The base case: markdown with no frontmatter, no kit and no links still
    /// draws, and what it leaves out is counted rather than vanishing.
    #[test]
    fn a_plain_markdown_repo_draws_without_any_types() {
        let l = base(&[
            "README.md", "docs/one.md", "docs/two.MD", "notes/x.markdown",
            "src/main.rs", "Cargo.toml", ".lex/COMPACT-ONTOLOGY.md",
        ]);
        assert_eq!(l.meta.node_count, 4);
        assert_eq!(l.meta.other_files, 2);
        assert_eq!(l.meta.machinery_files, 1);
        assert_eq!(l.meta.undated, 0, "every file had a commit, so none belongs on the rim");
        let names: Vec<&str> = l.meta.classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["docs/", "notes/", "(top level)"]);
        assert!(l.meta.docs.iter().all(|d| d.id.starts_with(FILE_PREFIX)));
    }

    /// A folder holding a large share of the repo is split, or the picture is
    /// one colour. lUX is 94% `Copia/`; W3BL0RD is 51% `Soul/`.
    #[test]
    fn a_folder_holding_most_of_the_repo_is_split_one_level_down() {
        let l = base(&[
            "Soul/Journal/a.md", "Soul/Journal/b.md", "Soul/Note/c.md",
            "Soul/Note/d.md", "Soul/e.md", "SOUL.md", "Harness/m.md",
        ]);
        let names: Vec<&str> = l.meta.classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Soul/Journal/", "Soul/Note/", "Harness/", "Soul/", "(top level)"]);

        // Just over a third splits; just under does not. 4 of 11 and 3 of 11.
        let l = base(&[
            "A/x/1.md", "A/x/2.md", "A/y/3.md", "A/y/4.md",
            "B/x/5.md", "B/x/6.md", "B/y/7.md",
            "C/8.md", "D/9.md", "E/10.md", "F/11.md",
        ]);
        let names: HashSet<&str> = l.meta.classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains("A/x/") && names.contains("A/y/"), "{names:?}");
        assert!(names.contains("B/") && !names.contains("B/x/"), "{names:?}");
    }

    /// More folders than colours: the tail shares one grey entry instead of
    /// two folders quietly sharing a hue.
    #[test]
    fn folders_past_the_palette_share_one_entry() {
        let paths: Vec<String> = (0..15).map(|i| format!("f{i:02}/doc.md")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let l = base(&refs);
        assert_eq!(l.meta.classes.len(), MAX_NAMED + 1);
        let rest = l.meta.classes.last().unwrap();
        assert_eq!((rest.name.as_str(), rest.count), ("(other folders)", 4));
        let colours: HashSet<&str> =
            l.meta.classes[..MAX_NAMED].iter().map(|c| c.color.as_str()).collect();
        assert_eq!(colours.len(), MAX_NAMED, "two named folders share a colour");
    }

    /// One view: a typed file is coloured by its type, an untyped one by its
    /// folder, and the folder split is measured over the untyped files alone.
    #[test]
    fn a_typed_file_takes_its_type_and_the_rest_keep_their_folder() {
        let paths = [
            "Soul/Note/a.md", "Soul/Note/b.md", "Soul/Journal/c.md",
            "Harness/Memory/m1.md", "Harness/Memory/m2.md", "Harness/x.md", "README.md",
        ];
        let mut history = HashMap::new();
        for (i, p) in paths.iter().enumerate() {
            history.insert(p.to_string(), PathHistory { first_sha: format!("c{i}"), commits: 1 });
        }
        let ords = (0..paths.len())
            .map(|i| OrdinalRow { id: format!("c{i}"), ord: (i + 1).to_string() })
            .collect();
        let ty = |f: &str, t: &str| TypeRow {
            file: format!("{FILE_PREFIX}{f}"),
            ty: format!("https://repolex.ai/ontology/soul/{t}"),
        };
        let types = vec![
            ty("Soul/Note/a.md", "Note"),
            ty("Soul/Note/b.md", "Note"),
            ty("Soul/Journal/c.md", "Journal"),
        ];
        let l = build_base(
            "g", "h", tree(&paths), &history, ords,
            vec![], vec![], vec![], types, vec![], vec![], None, None,
        );
        assert_eq!(l.meta.node_count, 7, "typing a file must never drop it");
        let names: Vec<&str> = l.meta.classes.iter().map(|c| c.name.as_str()).collect();
        // Harness/ is 3 of the 4 untyped files, so it splits one level down.
        assert_eq!(names, ["Note", "Journal", "Harness/Memory/", "Harness/", "(top level)"]);
        let colours: HashSet<&str> = l.meta.classes[..4].iter().map(|c| c.color.as_str()).collect();
        assert_eq!(colours.len(), 4, "a type and a folder share a colour");
    }
}
