// Mirrors the Rust types in src/registry.rs, src/repo.rs and src/supervisor.rs.
// Kept as literal unions rather than strings so that a renamed variant on the
// Rust side fails here at build time instead of silently matching nothing in
// a template.

export type Verdict = 'live' | 'path-missing' | 'no-lex-dir' | 'scratch'

/** Which clock a row's `recency` came from. Never drop this on the floor:
 *  46 of 50 registry entries have no `last_used`, so most rows are ordered by
 *  commit time, and a column that mixes two clocks silently is a lie. */
export type RecencySource = 'last-used' | 'head-commit' | 'unknown'

export type ServerState =
  | 'starting'
  | 'ready'
  | 'identity-mismatch'
  | 'unreachable'
  | 'exited'
  | 'failed'

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
  | { state: 'never-synced' }

export interface RepoProbe {
  path: string
  genesis_sha: string | null
  name: string
  /** False when `name` is a guess from the directory, not a declared name. */
  name_declared: boolean
  agent_name: string | null
  kit: string | null
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

export interface ServerStatus {
  path: string
  genesis_sha: string | null
  state: ServerState
  port: number | null
  requested_port: number
  pid: number | null
  www_dir: string | null
  last_seen_ms: number | null
  message: string | null
}
