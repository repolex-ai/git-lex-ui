// Mirrors the Rust types in src/registry.rs, src/repo.rs and src/supervisor.rs.
// Kept as literal unions rather than strings so that a renamed variant on the
// Rust side fails here at build time instead of silently matching nothing in
// a template.

export type Verdict = 'live' | 'path-missing' | 'no-lex-dir' | 'scratch'

/** Which clock a row's `recency` came from. Never drop this on the floor:
 *  46 of 50 registry entries have no `last_used`, so most rows are ordered by
 *  commit time, and a column that mixes two clocks silently is a lie. */
export type RecencySource = 'last-used' | 'head-commit' | 'unknown'

export interface Classified {
  path: string
  last_used: string | null
  verdict: Verdict
  reason: string
}

export type GraphFreshness =
  | { state: 'current'; sha: string }
  | { state: 'behind'; sha: string; commits: number | null }
  | { state: 'unplaceable' }
  /** The store was written after the spine that claims to describe it — so
   *  the spine's position is a claim about a store that no longer exists, and
   *  the last sync did not finish. No number is offered, because any number
   *  here would be confidently wrong. */
  | { state: 'spine-stale'; spine_sha: string }
  | { state: 'never-synced' }

/** What `git lex sync` is doing for one repo. */
export type SyncState =
  | { state: 'running'; started_ms: number }
  | { state: 'done'; ms: number; summary: string }
  | { state: 'failed'; ms: number; message: string }

export type RepoFamily =
  | { family: 'soul' }
  | { family: 'kitted'; kit: string }
  | { family: 'plain' }

export interface RepoProbe {
  path: string
  genesis_sha: string | null
  name: string
  /** False when `name` is a guess from the directory, not a declared name. */
  name_declared: boolean
  agent_name: string | null
  kit: string | null
  /** Soul, another kit, or plain markdown. git-lex's base case is a repo of
   *  markdown files with no kit at all — the list segments on this so that
   *  case stays visible on a machine that is mostly souls. */
  family: RepoFamily
  optional_kits: string[]
  head_sha: string | null
  head_time: string | null
  commit_count: number | null
  recency: string | null
  recency_source: RecencySource
  /** Where the persisted graph sits relative to HEAD. Read from the spine
   *  filename. `unplaceable` and `never-synced` are NOT "fine" — they mean
   *  the question could not be answered from disk, and must not render as
   *  current. */
  graph: GraphFreshness
  has_www: boolean
  warnings: string[]
}

export interface PruneReport {
  removed: Classified[]
  kept: number
  arrived_during_prune: number
  backup_path: string | null
  rewrote_file: boolean
}

export interface Counts {
  shown: number
  dropped_total: number
  dropped_path_missing: number
  dropped_no_lex: number
  dropped_scratch: number
  recency_from_commit: number
}

export interface ReposResponse {
  repos: RepoProbe[]
  dropped: Classified[]
  prune: PruneReport | null
  registry_path: string | null
  registry_format: string | null
  counts: Counts
}

/** One repo as `gitlexd` reports it.
 *
 *  `synced_to` is the LIVE answer to how far a graph has fallen behind: the
 *  commit the store was actually built up to, from the process that built it.
 *  Everything else on disk is a trace of that work rather than a statement of
 *  it — and a trace gets left just as readily by someone merely OPENING the
 *  store, which is how six healthy repos wore a warning on 2026-09-22. */
export interface Soul {
  genesis: string
  name: string | null
  path: string
  synced_to: string | null
  syncing: boolean
  last_error: string | null
  open_error: string | null
  last_sync_ms: number | null
  syncs: number | null
}

/** The state of the one data feed on the machine.
 *
 *  There is one of these, where there used to be one row per repo behind a
 *  supervisor. A page that cannot draw has exactly one thing to check.
 *
 *  `reachable: false` with an empty `souls` is not the same as a daemon that
 *  holds nothing, which is why the flag is carried separately from the list. */
export interface DaemonStatus {
  reachable: boolean
  port: number
  health: {
    ok: boolean
    pid: number | null
    port: number | null
    souls: number | null
    syncing: number | null
    uptime_secs: number | null
    version: string | null
  } | null
  souls: Soul[]
  message: string | null
}

/** One document's text, or the reason it could not be read.
 *
 *  `error` non-null is not necessarily a fault: a document can be recorded in
 *  the graph and legitimately absent from disk, because the graph records
 *  history and history includes deletions. The panel distinguishes the two. */
export interface FileText {
  path: string
  bytes: number
  text: string | null
  error: string | null
}
