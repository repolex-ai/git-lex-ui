import { NodeState, type LayoutOffsets } from './renderer'

export interface ClassInfo { uri: string; name: string; count: number; color: string }
export interface PredicateInfo { uri: string; name: string; count: number }
export interface DocMeta {
  id: string
  label: string
  class: number
  born: number | null
  events: number
}
export interface Dropped {
  reason: string
  count: number
  examples: string[]
  /** Which predicates produced these, largest first. One total hides the
   *  difference between systematic shape and scattered rot. */
  by_predicate?: [string, number][]
}

export interface LayoutMeta {
  genesis_sha: string
  head_sha: string
  store_head: string | null
  commits_behind: number | null
  built_at_ms: number
  node_count: number
  edge_count: number
  turns: number
  classes: ClassInfo[]
  predicates: PredicateInfo[]
  docs: DocMeta[]
  turn_dates: (string | null)[]
  undated: number
  first_ordinal: number | null
  last_ordinal: number | null
  file_subjects: number
  folded_files: number
  unbridged_things: number
  titled: number
  dropped: Dropped[]
  offsets: LayoutOffsets
}

export interface Loaded {
  meta: LayoutMeta
  buffer: ArrayBuffer
  source: string
}

export async function loadLayout(genesis: string): Promise<Loaded> {
  const mr = await fetch(`/api/layout/${genesis}`)
  if (!mr.ok) throw new Error(await mr.text())
  const source = mr.headers.get('x-layout-source') ?? ''
  const meta: LayoutMeta = await mr.json()
  const dr = await fetch(`/api/layout/${genesis}/data`)
  if (!dr.ok) throw new Error(await dr.text())
  return { meta, buffer: await dr.arrayBuffer(), source }
}

/** Undirected adjacency, built once per soul. Used for the neighbourhood
 *  layout and for the inspector's link lists, so both answer from the same
 *  edge set the picture is drawn from. */
export class Adjacency {
  readonly out: number[][] = []
  readonly inc: number[][] = []

  constructor(n: number, edges: Uint32Array) {
    for (let i = 0; i < n; i++) {
      this.out.push([])
      this.inc.push([])
    }
    // Neighbours are UNIQUE, and that is load-bearing rather than tidy.
    //
    // Two documents can be joined by more than one edge — the same target
    // reached through `relatedToId` in the frontmatter AND through a body
    // link in the prose, which is the normal case for a document that
    // declares a reference and then also writes about it. Left as duplicates,
    // a keyed `{#each}` over a neighbour list throws `each_key_duplicate`,
    // which ABORTS the update: the panel renders half-built, showing "7 out"
    // in its header and "0" in the section below it, and the canvas never
    // draws at all. One exception, three symptoms, none of them looking like
    // a duplicate key.
    //
    // A neighbour is a document you can travel to. How many edges lead there
    // is a fact about the edges, and it is counted where edges are counted.
    const seenOut = new Set<number>()
    const seenInc = new Set<number>()
    for (let e = 0; e < edges.length; e += 2) {
      const a = edges[e]
      const b = edges[e + 1]
      if (a >= n || b >= n) continue
      const ko = a * n + b
      if (!seenOut.has(ko)) {
        seenOut.add(ko)
        this.out[a].push(b)
      }
      const ki = b * n + a
      if (!seenInc.has(ki)) {
        seenInc.add(ki)
        this.inc[b].push(a)
      }
    }
  }

  degree(i: number): number {
    return this.out[i].length + this.inc[i].length
  }

  /** Nodes within `hops` of `start`, with the hop distance of each. */
  neighbourhood(start: number, hops: number): Map<number, number> {
    const seen = new Map<number, number>([[start, 0]])
    let frontier = [start]
    for (let d = 1; d <= hops; d++) {
      const next: number[] = []
      for (const i of frontier) {
        for (const j of [...this.out[i], ...this.inc[i]]) {
          if (!seen.has(j)) {
            seen.set(j, d)
            next.push(j)
          }
        }
      }
      frontier = next
      if (!frontier.length) break
    }
    return seen
  }
}

/** The spiral track as a polyline, for drawing under the dots. */
export function trackPoints(turns: number): Float32Array {
  const steps = Math.max(400, turns * 400)
  const out = new Float32Array((steps + 1) * 2)
  for (let i = 0; i <= steps; i++) {
    const t = i / steps
    const th = 2 * Math.PI * turns * t
    const r = 0.12 + 0.86 * t
    out[i * 2] = Math.cos(th) * r
    out[i * 2 + 1] = Math.sin(th) * r
  }
  return out
}

/**
 * Concentric-ring layout around one node.
 *
 * Deliberately not a force simulation. The point of this view is to answer
 * "what is this document connected to, and how far away" — a question with an
 * exact answer that should not wobble, settle, or come out differently on a
 * second look. Hop distance is the radius; everything at the same distance
 * shares a ring, ordered by degree so the busiest neighbours are adjacent and
 * comparable rather than scattered.
 *
 * Nodes outside the neighbourhood are parked far off-stage rather than left
 * where the spiral put them, which would read as connections that are not
 * there.
 */
export function neighbourhoodPositions(
  n: number,
  adj: Adjacency,
  centre: number,
  hops: number,
): { positions: Float32Array; members: Map<number, number> } {
  const members = adj.neighbourhood(centre, hops)
  const positions = new Float32Array(n * 2).fill(1e6)

  const byRing = new Map<number, number[]>()
  for (const [i, d] of members) {
    const b = byRing.get(d)
    if (b) b.push(i)
    else byRing.set(d, [i])
  }

  positions[centre * 2] = 0
  positions[centre * 2 + 1] = 0

  for (const [d, ring] of byRing) {
    if (d === 0) continue
    ring.sort((a, b) => adj.degree(b) - adj.degree(a) || a - b)
    const r = d / (hops + 0.35)
    for (let k = 0; k < ring.length; k++) {
      // Offset each ring's start angle so successive rings do not line up
      // into spokes, which read as structure that is not in the data.
      const th = (k / ring.length) * 2 * Math.PI + d * 0.6
      positions[ring[k] * 2] = Math.cos(th) * r
      positions[ring[k] * 2 + 1] = Math.sin(th) * r
    }
  }
  return { positions, members }
}

/**
 * The edges to draw, after filtering.
 *
 * Both filters live here so the count shown in the bar and the lines on the
 * stage can never disagree — they are the same array.
 *
 * `members` restricts to a neighbourhood; `visiblePredicates` restricts by
 * kind of link. The second matters more than it sounds: on lUX one predicate
 * accounts for 14,529 of 18,131 links, so without it the picture is a single
 * predicate's hairball with everything else buried underneath.
 */
export function edgesToDraw(
  edges: Uint32Array,
  edgePredicates: Uint16Array,
  visiblePredicates: Set<number> | null,
  members: Map<number, number> | null,
  states: Uint8Array | null,
): Uint32Array {
  const keep: number[] = []
  for (let e = 0, k = 0; e < edges.length; e += 2, k++) {
    const a = edges[e]
    const b = edges[e + 1]
    if (visiblePredicates && !visiblePredicates.has(edgePredicates[k])) continue
    if (members && (!members.has(a) || !members.has(b))) continue
    // A link to a node that has been filtered off the stage would draw as a
    // line into empty space.
    if (states && (states[a] === 0 || states[b] === 0)) continue
    keep.push(a, b)
  }
  return new Uint32Array(keep)
}

export interface StateInput {
  n: number
  docs: DocMeta[]
  /** Class indices currently switched on. */
  visibleClasses: Set<number>
  /** Lowercased search text; empty means no search. */
  search: string
  selected: number | null
  /** When set, only these nodes are shown at all. */
  restrictTo: Map<number, number> | null
}

/**
 * Fold every filter into one state byte per node.
 *
 * A class switched off is Hidden — genuinely removed, not merely faint,
 * because a filter that leaves things pickable is a filter you cannot trust.
 * A search that matches nothing leaves everything Normal rather than blanking
 * the stage: an empty result is a fact about the search, and it is reported in
 * the rail, not by an empty screen with no explanation.
 */
export function computeStates(inp: StateInput): { states: Uint8Array; matches: number[] } {
  const states = new Uint8Array(inp.n)
  const matches: number[] = []
  const searching = inp.search.length > 0

  for (let i = 0; i < inp.n; i++) {
    const d = inp.docs[i]
    if (!inp.visibleClasses.has(d.class)) {
      states[i] = NodeState.Hidden
      continue
    }
    if (inp.restrictTo && !inp.restrictTo.has(i)) {
      states[i] = NodeState.Hidden
      continue
    }
    let s: number = NodeState.Normal
    if (searching) {
      if (d.label.toLowerCase().includes(inp.search) || d.id.toLowerCase().includes(inp.search)) {
        s = NodeState.Marked
        matches.push(i)
      } else {
        s = NodeState.Dim
      }
    }
    states[i] = s
  }

  if (inp.selected !== null && states[inp.selected] !== NodeState.Hidden) {
    states[inp.selected] = NodeState.Selected
  }
  return { states, matches }
}
