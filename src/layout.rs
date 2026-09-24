//! The whole-repo spiral layout, computed server-side.
//!
//! Two readings share it (see [`View`]): **base**, every markdown file by
//! folder and dated by git, which works for any repo; and **typed**,
//! documents the kit has typed, coloured by type. They differ only in which
//! documents they collect and how they group them — `place_and_pack` below is
//! the one piece that draws both.
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

/// MIN(ordinal) over every fact ever asserted about a subject is the commit
/// the document was born in. `ordinalDerived` is the ordering authority:
/// author dates tie, and they lie under rebase.
pub fn q_born() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         PREFIX g2: <https://repolex.ai/ontology/git-lex/git2/>
         PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
         SELECT ?s (MIN(?ord) AS ?born) (COUNT(?e) AS ?events) WHERE {{
             GRAPH <{ONE_GRAPH}> {{ ?e rdf:reifies <<( ?s ?p ?o )>> ; gl:assertedIn ?c }}
             GRAPH <{COMMITS}> {{ ?c g2:ordinalDerived ?ord }}
         }} GROUP BY ?s"
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

/// Every typed subject in the now view, with a display label.
///
/// Includes orphans that nothing links to — a document nobody linked is
/// still a document, and dropping it would quietly shrink the census.
pub fn q_nodes() -> String {
    format!(
        "PREFIX gl: <https://repolex.ai/ontology/git-lex/>
         SELECT ?id ?type ?label WHERE {{
             GRAPH <{NOW}> {{
                 ?id a ?type .
                 OPTIONAL {{ ?id gl:name ?n }}
                 BIND(COALESCE(?n, REPLACE(STR(?id), \"^.*/\", \"\")) AS ?label)
             }}
         }}"
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

/// Which reading of a repo a layout is.
///
/// **Base** works for any git-lex repo, kit or no kit: every markdown file
/// the store's file tree lists, placed by the commit git says it first
/// appeared in, coloured by folder. It needs nothing from frontmatter, which
/// is the point — git-lex's base case is a folder of markdown files, and a
/// view that only draws typed documents draws nothing for that case. Decided
/// with @goodlux on 2026-09-17, as the layer every kit view sits on top of.
///
/// **Typed** is the original view: documents from the store's `now` view,
/// coloured by class. It needs a kit to have typed something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Base,
    Typed,
}

impl View {
    pub fn parse(s: &str) -> Option<View> {
        match s {
            "base" => Some(View::Base),
            "typed" => Some(View::Typed),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            View::Base => "base",
            View::Typed => "typed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutMeta {
    pub view: View,
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
    /// Subjects typed `gl:File` in the store, before folding.
    pub file_subjects: usize,
    /// Files that carry a Thing, and so are drawn as that Thing rather than
    /// as a file. `file_subjects - folded_files` is the number of documents
    /// that are *only* a file — which is what the File entry in `classes`
    /// counts, and why that number is smaller than the store's file count.
    /// Two true numbers under one label is how a reader gets stuck, so both
    /// are carried and the page states the relation.
    pub folded_files: usize,
    /// Things whose `gl:fileId` names a file that is not itself in the node
    /// set. They are still drawn, under their own class.
    pub unbridged_things: usize,
    /// Base view only: files in the tree that are not markdown, and so are
    /// not drawn. Counted so a code repo does not read as a tiny one.
    pub other_files: usize,
    /// Base view only: markdown under `.lex/`, which is git-lex's own
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
pub struct NodeRow {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub ty: String,
}

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
pub struct BornRow {
    pub s: String,
    pub born: String,
    pub events: String,
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

pub fn build(
    genesis_sha: &str,
    head_sha: &str,
    nodes: Vec<NodeRow>,
    edges: Vec<EdgeRow>,
    link_born_rows: Vec<LinkBornRow>,
    aliases: Vec<AliasRow>,
    born_rows: Vec<BornRow>,
    date_rows: Vec<DateRow>,
    label_rows: Vec<LabelRow>,
    store_head: Option<String>,
    commits_behind: Option<usize>,
) -> Layout {
    // Best title per subject: lowest rank wins, so a declared Thing-plane
    // name beats a file-plane one.
    let mut best_label: HashMap<&str, (u8, &str)> = HashMap::new();
    for r in &label_rows {
        let rank: u8 = r.rank.parse().unwrap_or(9);
        let e = best_label.entry(r.s.as_str()).or_insert((rank, r.label.as_str()));
        if rank < e.0 {
            *e = (rank, r.label.as_str());
        }
    }
    // --- fold the Thing plane onto the File plane -------------------------
    let file_of_thing: HashMap<&str, &str> = aliases
        .iter()
        .map(|a| (a.thing.as_str(), a.file.as_str()))
        .collect();
    let fold = |id: &str| -> String {
        file_of_thing.get(id).map(|s| s.to_string()).unwrap_or_else(|| id.to_string())
    };

    let mut born_of: HashMap<&str, i64> = HashMap::new();
    let mut events_of: HashMap<&str, u32> = HashMap::new();
    for r in &born_rows {
        if let Ok(b) = r.born.parse::<i64>() {
            born_of.insert(r.s.as_str(), b);
        }
        events_of.insert(r.s.as_str(), r.events.parse().unwrap_or(0));
    }

    struct Agg {
        id: String,
        types: Vec<String>,
        labels: HashMap<String, String>,
        twins: HashSet<String>,
    }
    let file_subjects = nodes.iter().filter(|r| r.ty == GL_FILE).count();
    let typed_files: HashSet<&str> = nodes
        .iter()
        .filter(|r| r.ty == GL_FILE)
        .map(|r| r.id.as_str())
        .collect();
    let unbridged_things = aliases
        .iter()
        .filter(|a| !typed_files.contains(a.file.as_str()))
        .count();

    let mut by_subject: HashMap<String, Agg> = HashMap::new();
    for r in &nodes {
        let id = fold(&r.id);
        let e = by_subject.entry(id.clone()).or_insert_with(|| Agg {
            id: id.clone(),
            types: vec![],
            labels: HashMap::new(),
            twins: HashSet::new(),
        });
        e.types.push(r.ty.clone());
        if let Some(l) = &r.label {
            e.labels.insert(r.ty.clone(), l.clone());
        }
        e.twins.insert(r.id.clone());
    }

    let docs: Vec<Placed> = by_subject
        .into_values()
        .filter_map(|e| {
            // The most specific type wins: a document that is both a File and
            // a Note is a Note. File is the transitory plane — substrate, not
            // identity — so it is only the answer when it is the only answer.
            let ty = e
                .types
                .iter()
                .find(|t| t.as_str() != GL_FILE)
                .cloned()
                .or_else(|| e.types.first().cloned())?;
            // A real title, from either plane, beats the slug that
            // `/api/viz/nodes` hands back. Checked across every twin, because
            // the title commonly sits on the file while the type sits on the
            // Thing.
            let resolved = e
                .twins
                .iter()
                .filter_map(|t| best_label.get(t.as_str()).map(|(r, l)| (*r, *l)))
                .min_by_key(|(r, _)| *r)
                .map(|(_, l)| l.to_string());
            let resolved_present = resolved.is_some();
            // Fall back to the FOLDED id's last segment, not to the slug that
            // came back on the node row.
            //
            // These are usually the same string, and in one case they are
            // not: SOUL.md. Its Thing is minted from the genesis sha, so its
            // slug is a 40-character hash — and on the single document whose
            // job is to say WHICH SOUL THIS IS, the untitled fallback
            // rendered as `e3d71e7f0e022e54...`. That is the hex-string
            // symptom itself, surviving the very fix that was meant to end
            // it, on the worst possible document. The folded id is the file,
            // so its last segment is `SOUL.md`, which is a name a person can
            // read.
            let label = resolved.unwrap_or_else(|| short_name(&e.id));
            // A document is as old as the earliest fact about EITHER of its
            // subjects: a Thing minted later than its File is the same
            // document arriving, not a new one.
            let born = e.twins.iter().filter_map(|t| born_of.get(t.as_str()).copied()).min();
            let events = e
                .twins
                .iter()
                .map(|t| events_of.get(t.as_str()).copied().unwrap_or(0))
                .sum();
            Some(Placed {
                id: e.id,
                group: ty,
                titled: resolved_present,
                label,
                born,
                events,
            })
        })
        .collect();

    // --- classes ----------------------------------------------------------
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for d in &docs {
        *counts.entry(d.group.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    // Colour order is meaning order, not size order. On a soul with many
    // unclassed documents, File is also the biggest class — sorting by size
    // hands the loudest colour to the least meaningful thing on screen. It
    // goes last, and it goes grey.
    let mut classes: Vec<ClassInfo> = ranked
        .iter()
        .filter(|(u, _)| *u != GL_FILE)
        .enumerate()
        .map(|(i, (uri, count))| ClassInfo {
            uri: uri.to_string(),
            name: short_name(uri),
            count: *count,
            color: color_for(i),
        })
        .collect();
    for (uri, count) in ranked.iter().filter(|(u, _)| *u == GL_FILE) {
        classes.push(ClassInfo {
            uri: uri.to_string(),
            // Not "File". After folding, this counts documents that are ONLY
            // a file — the ones with no Thing speaking through them. Labelled
            // "File" it sat on the same page as the store's raw file count
            // under the same word, 68 against 135, with nothing saying they
            // were answers to different questions.
            name: "File only".to_string(),
            count: *count,
            color: "#b9b9bd".to_string(),
        });
    }
    let file_only = classes.iter().find(|c| c.uri == GL_FILE).map_or(0, |c| c.count);
    place_and_pack(
        Shared { genesis_sha, head_sha, store_head, commits_behind },
        docs,
        classes,
        &fold,
        edges,
        link_born_rows,
        date_rows,
        Census {
            view: View::Typed,
            file_subjects,
            folded_files: file_subjects.saturating_sub(file_only),
            unbridged_things,
            other_files: 0,
            machinery_files: 0,
        },
    )
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
/// Folders past this many get one shared grey entry, so the palette is never
/// reused for two different folders.
const MAX_FOLDERS: usize = 11;

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

    // Folder for each path: the top folder, unless that folder is most of
    // the repo, in which case its subfolders.
    let mut top_counts: HashMap<&str, usize> = HashMap::new();
    for p in &paths {
        *top_counts.entry(top(p)).or_insert(0) += 1;
    }
    let split: HashSet<&str> = top_counts
        .iter()
        .filter(|(k, c)| !k.is_empty() && **c as f64 > paths.len() as f64 * SPLIT_SHARE)
        .map(|(k, _)| *k)
        .collect();
    let folder_of = |p: &str| -> String {
        let t = top(p);
        if split.contains(t) { two_deep(p).to_string() } else { t.to_string() }
    };

    let mut folder_counts: HashMap<String, usize> = HashMap::new();
    for p in &paths {
        *folder_counts.entry(folder_of(p)).or_insert(0) += 1;
    }
    let mut ranked: Vec<(String, usize)> = folder_counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let named: HashSet<String> = ranked
        .iter()
        .filter(|(f, _)| !f.is_empty())
        .take(MAX_FOLDERS)
        .map(|(f, _)| f.clone())
        .collect();
    const ROOT: &str = "(top level)";
    const REST: &str = "(other folders)";
    let legend_key = |p: &str| -> String {
        let f = folder_of(p);
        if f.is_empty() {
            ROOT.to_string()
        } else if named.contains(&f) {
            f
        } else {
            REST.to_string()
        }
    };

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
    // Real folders first by size; the top level and the catch-all go last,
    // in grey, because neither is a place anyone chose.
    let rank = |k: &str| match k {
        ROOT => 1,
        REST => 2,
        _ => 0,
    };
    order.sort_by(|a, b| rank(a.0).cmp(&rank(b.0)).then(b.1.cmp(&a.1)).then(a.0.cmp(b.0)));
    let classes: Vec<ClassInfo> = order
        .iter()
        .enumerate()
        .map(|(i, (k, c))| ClassInfo {
            uri: k.to_string(),
            name: if rank(k) == 0 { format!("{k}/") } else { k.to_string() },
            count: *c,
            color: match rank(k) {
                0 => color_for(i),
                1 => "#8d8d93".to_string(),
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
            view: View::Base,
            file_subjects: 0,
            folded_files: 0,
            unbridged_things: 0,
            other_files,
            machinery_files,
        },
    )
}

/// One document, ready to be placed. Both views reduce to this: the typed
/// view groups by class, the base view by folder, and nothing below that
/// point knows which one it was given.
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

/// View-specific counts carried through to the metadata unchanged.
struct Census {
    view: View,
    file_subjects: usize,
    folded_files: usize,
    unbridged_things: usize,
    other_files: usize,
    machinery_files: usize,
}

/// Place documents on the spiral, resolve links, and pack the result.
///
/// Everything here is the same for every view: angle is the birth commit,
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
            view: census.view,
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
            file_subjects: census.file_subjects,
            folded_files: census.folded_files,
            unbridged_things: census.unbridged_things,
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

    fn rows<T: serde::de::DeserializeOwned>(dir: &str, name: &str) -> Vec<T> {
        let text = std::fs::read_to_string(format!("{dir}/{name}")).expect("fixture");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        serde_json::from_value(v["results"].clone()).expect("rows")
    }

    /// Build a real soul's layout from captured server responses.
    ///
    /// Run against the largest soul on this machine — ~12.9k node rows
    /// folding through 6.5k aliases — because every constraint in this module
    /// is about what happens at scale, and a 147-document soul exercises none
    /// of them. Set `GIT_LEX_UI_FIXTURES` to a directory holding the five
    /// captured responses; skipped when it is not set, so the suite still
    /// runs on a machine without them.
    #[test]
    fn builds_a_large_soul() {
        let Ok(dir) = std::env::var("GIT_LEX_UI_FIXTURES") else {
            eprintln!("GIT_LEX_UI_FIXTURES not set — skipping the scale test");
            return;
        };

        let nodes: Vec<NodeRow> = rows(&dir, "lux_nodes.json");
        let edges: Vec<EdgeRow> = rows(&dir, "lux_edges.json");
        let aliases: Vec<AliasRow> = rows(&dir, "lux_alias.json");
        let born: Vec<BornRow> = rows(&dir, "lux_born.json");
        let dates: Vec<DateRow> = rows(&dir, "lux_dates.json");
        let labels: Vec<LabelRow> = Vec::new();
        let node_rows = nodes.len();

        let t0 = std::time::Instant::now();
        let l = build("genesis", "head", nodes, edges, Vec::new(), aliases, born, dates, labels, None, None);
        let elapsed = t0.elapsed();

        eprintln!(
            "{node_rows} node rows -> {} documents, {} edges, {} turns, in {elapsed:?} ({} bytes packed)",
            l.meta.node_count, l.meta.edge_count, l.meta.turns, l.meta.offsets.total
        );

        assert!(l.meta.node_count > 5000, "folding lost documents");
        assert_eq!(l.meta.docs.len(), l.meta.node_count);
        assert_eq!(l.meta.offsets.positions_bytes, l.meta.node_count * 8);
        assert_eq!(l.meta.offsets.colors_bytes, l.meta.node_count * 3);
        assert_eq!(l.meta.offsets.edges_bytes, l.meta.edge_count * 8);
        assert_eq!(l.meta.offsets.edge_predicates_bytes, l.meta.edge_count * 2);
        // Every drawn edge must name a predicate that exists in the table,
        // or the legend and the picture are describing different graphs.
        let at = l.meta.offsets.edge_predicates;
        for i in 0..l.meta.edge_count {
            let v = u16::from_le_bytes(l.data[at + i * 2..at + i * 2 + 2].try_into().unwrap());
            assert!(
                (v as usize) < l.meta.predicates.len(),
                "edge predicate index {v} is not in the predicate table"
            );
        }
        assert_eq!(
            l.meta.predicates.iter().map(|p| p.count).sum::<usize>(),
            l.meta.edge_count,
            "predicate counts must account for every drawn edge"
        );
        assert_eq!(l.data.len(), l.meta.offsets.total);

        // Every edge index must address a real node. An out-of-range index is
        // not a wrong picture, it is a GPU read past the end of a buffer.
        let idx = l.meta.offsets.edges;
        for i in 0..l.meta.edge_count * 2 {
            let at = idx + i * 4;
            let v = u32::from_le_bytes(l.data[at..at + 4].try_into().unwrap());
            assert!((v as usize) < l.meta.node_count, "edge index {v} out of range");
        }

        // The layout must be a pure function of its input: same soul in, same
        // picture out, or it is not something two people can talk about.
        let l2 = build(
            "genesis",
            "head",
            rows(&dir, "lux_nodes.json"),
            rows(&dir, "lux_edges.json"),
            Vec::new(),
            rows(&dir, "lux_alias.json"),
            rows(&dir, "lux_born.json"),
            rows(&dir, "lux_dates.json"),
            Vec::new(),
            None,
            None,
        );
        assert_eq!(l.data, l2.data, "layout is not deterministic");
    }

    /// Every document must land within the spiral's own track, offset only
    /// along the normal. Spreading a same-commit cohort along the ANGLE would
    /// make an afternoon's import read as weeks of work, and the failure is
    /// invisible in a screenshot — it just looks like a busier soul.
    #[test]
    fn same_commit_documents_spread_across_the_track_not_along_it() {
        // 40 documents, all born in one commit, plus one earlier and one later
        // so the spiral has a span to place them on.
        let mut nodes = vec![];
        let mut born = vec![];
        for i in 0..40 {
            let id = format!("https://example/File/doc{i}.md");
            nodes.push(NodeRow {
                id: id.clone(),
                label: Some(format!("doc{i}")),
                ty: "https://repolex.ai/ontology/soul/Note".to_string(),
            });
            born.push(BornRow { s: id, born: "50".into(), events: "1".into() });
        }
        for (i, o) in [("first", "1"), ("last", "100")] {
            let id = format!("https://example/File/{i}.md");
            nodes.push(NodeRow {
                id: id.clone(),
                label: Some(i.to_string()),
                ty: "https://repolex.ai/ontology/soul/Note".to_string(),
            });
            born.push(BornRow { s: id, born: o.into(), events: "1".into() });
        }

        let l = build("g", "h", nodes, vec![], vec![], vec![], born, vec![], vec![], None, None);
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
        build_base("g", "h", tree(paths), &history, ords, vec![], vec![], vec![], vec![], vec![], None, None)
    }

    /// The base case: markdown with no frontmatter, no kit and no links still
    /// draws, and what it leaves out is counted rather than vanishing.
    #[test]
    fn a_plain_markdown_repo_draws_without_any_types() {
        let l = base(&[
            "README.md", "docs/one.md", "docs/two.MD", "notes/x.markdown",
            "src/main.rs", "Cargo.toml", ".lex/COMPACT-ONTOLOGY.md",
        ]);
        assert_eq!(l.meta.view, View::Base);
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
        assert_eq!(l.meta.classes.len(), MAX_FOLDERS + 1);
        let rest = l.meta.classes.last().unwrap();
        assert_eq!((rest.name.as_str(), rest.count), ("(other folders)", 4));
        let colours: HashSet<&str> =
            l.meta.classes[..MAX_FOLDERS].iter().map(|c| c.color.as_str()).collect();
        assert_eq!(colours.len(), MAX_FOLDERS, "two named folders share a colour");
    }
}
