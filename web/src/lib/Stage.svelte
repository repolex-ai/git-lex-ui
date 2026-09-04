<script lang="ts">
  import { GraphRenderer, NodeState, type View } from './renderer'
  import type { LayoutMeta, DocMeta } from './graph'
  import DocPanel from './DocPanel.svelte'
  import type { FileText } from './types'

  interface Props {
    meta: LayoutMeta | null
    buffer: ArrayBuffer | null
    states: Uint8Array | null
    track: Float32Array | null
    positions: Float32Array | null
    edgeSubset: Uint32Array | null
    selected: number | null
    view: 'spiral' | 'neighbourhood'
    centreOn: number | null
    onselect: (i: number | null) => void
    onready: (r: GraphRenderer) => void
    /** The document panel floats over the canvas, so it is rendered here
     *  rather than by the app shell — `.canvas-wrap` is the positioned
     *  ancestor it anchors to, and a sibling of the stage would have become
     *  a fourth column in the app's three-column grid. */
    docOpen: boolean
    docFile: FileText | null
    docLoading: boolean
    ondocclose: () => void
  }
  let {
    meta, buffer, states, track, positions, edgeSubset,
    selected, view, centreOn, onselect, onready,
    docOpen, docFile, docLoading, ondocclose,
  }: Props = $props()

  let canvas = $state<HTMLCanvasElement | null>(null)
  let hover = $state<{ doc: DocMeta; x: number; y: number } | null>(null)
  let showEdges = $state(true)
  let err = $state<string | null>(null)

  // $state, not a plain let: the effects below depend on the renderer
  // existing. Held in a non-reactive binding they run once, before it is
  // built, and then never again — which is how the spiral track went
  // missing while colours and positions (whose own inputs kept changing)
  // appeared to work.
  let r = $state<GraphRenderer | null>(null)
  let camera: View = { scale: 0.92, x: 0, y: 0 }
  let dpr = 1
  let raf = 0
  /** Set when a new soul loads; cleared by the first frame that has both the
   *  laid-out canvas and the track to measure. */
  let needsFit = false

  /** How close a pointer must be to a node, in layout units before scale.
   *  Deliberately larger than a dot: the dots are half the size they were,
   *  and a target you have to hit exactly is a worse target than a small
   *  one you can hit approximately. */
  const PICK_RADIUS = 0.028

  function frame() {
    raf = 0
    if (!r) return
    r.resize(dpr)
    // Fit on the first frame that can actually measure: the canvas has a
    // laid-out size and the track has arrived. Doing it here rather than at
    // construction is what makes the frame include the spiral.
    if (needsFit && canvas && canvas.clientHeight > 0) {
      camera = r.fitView(canvas.clientWidth / canvas.clientHeight)
      needsFit = false
    }
    r.draw(camera, dpr, showEdges)
  }
  function invalidate() {
    if (!raf) raf = requestAnimationFrame(frame)
  }

  // Build the renderer whenever a new soul's buffer arrives.
  $effect(() => {
    const m = meta
    const b = buffer
    if (!m || !b || !canvas) {
      r = null
      return
    }
    try {
      dpr = Math.min(window.devicePixelRatio || 1, 2)
      const next = new GraphRenderer(canvas, b, m.offsets, m.node_count)
      // Frame the content rather than opening at a fixed zoom. One scale for
      // every soul meant a 51-document soul opened as a dot and a 7,651-one
      // overflowed, so the first move was always to fix the view.
      //
      // Deferred rather than done here: the track is uploaded by a later
      // effect, and it reaches further than the nodes do. Fitting now frames
      // the dots and lets the spiral run off three edges — which is exactly
      // what it did on the first attempt.
      camera = { scale: 0.92, x: 0, y: 0 }
      needsFit = true
      r = next
      onready(next)
      err = null
      invalidate()
    } catch (e) {
      err = e instanceof Error ? e.message : String(e)
      r = null
    }
  })

  $effect(() => { if (r && states) { r.setStates(states); invalidate() } })
  $effect(() => { if (r && positions) { r.setPositions(positions); invalidate() } })
  $effect(() => {
    if (!r) return
    r.setTrack(track ?? new Float32Array(0))
    needsFit = true
    invalidate()
  })
  // --- chronological replay ----------------------------------------------
  //
  // Links appear in the order they were actually created. This is not
  // inferred from the ages of the two documents — git-lex reifies every
  // statement and records the commit that asserted it, so each link carries
  // its own birthday and the ordering is read rather than guessed.
  //
  // A link whose birthday the store does not record is held back until the
  // very end rather than shown at ordinal 0. Zero would mean "this existed
  // from the beginning", which is a confident claim about history and, for a
  // link we know nothing about, the wrong one.
  let playhead = $state<number | null>(null)
  let playing = $state(false)
  let playRaf = 0

  // The sweep runs over the LINKS' own range, not the documents'.
  //
  // On W3BL0RD the first document is born at commit 60 and the first link is
  // asserted at 130, so sweeping the document range spent the first 40% of
  // the animation showing nothing happen. Measured before changing it: 194
  // links across 28 distinct commits, 130 to 184.
  const linkRange = $derived.by(() => {
    if (!r || r.edgeBorn.length === 0) return null
    let lo = Infinity
    let hi = -Infinity
    for (const b of r.edgeBorn) {
      if (b === 0xffffffff) continue
      if (b < lo) lo = b
      if (b > hi) hi = b
    }
    return Number.isFinite(lo) && hi > lo ? { lo, hi } : null
  })
  // One less than the first link's commit, so the replay opens on an empty
  // graph. Starting exactly at `lo` put 49 of W3BL0RD's 194 links on screen
  // in the first frame — that commit really did assert 49 at once, and the
  // burst reads as history rather than as a slow start only if you see the
  // moment before it.
  const FIRST = $derived(linkRange ? linkRange.lo - 1 : (meta?.first_ordinal ?? 0))
  const LAST = $derived(linkRange?.hi ?? meta?.last_ordinal ?? 0)
  /** Seconds for a full sweep, whatever the repo's length. A soul with 3,500
   *  commits and one with 180 both want to be watched, not waited out. */
  const SWEEP_MS = 14000

  /** Edges visible now: the predicate filter, then the playhead. */
  const timeFiltered = $derived.by(() => {
    const base = edgeSubset ?? r?.edges ?? null
    if (!r || playhead === null || !base) return base
    const out: number[] = []
    for (let k = 0; k < base.length; k += 2) {
      const born = r.edgeBorn[k / 2]
      if (born !== 0xffffffff && born <= playhead) {
        out.push(base[k], base[k + 1])
      }
    }
    return new Uint32Array(out)
  })

  $effect(() => {
    if (!r) return
    r.setEdgeSubset(timeFiltered ?? r.edges)
    invalidate()
  })

  function stopPlay() {
    playing = false
    if (playRaf) cancelAnimationFrame(playRaf)
    playRaf = 0
  }

  function togglePlay() {
    if (playing) {
      stopPlay()
      playhead = null
      return
    }
    if (LAST <= FIRST) return
    playing = true
    const t0 = performance.now()
    const step = (t: number) => {
      const f = Math.min(1, (t - t0) / SWEEP_MS)
      playhead = FIRST + Math.round(f * (LAST - FIRST))
      if (f >= 1) {
        // Land on "everything", including the links with no recorded
        // birthday, so the end of the replay is the same picture as before
        // it started. A replay that ends somewhere other than the truth
        // would quietly become a filter.
        stopPlay()
        playhead = null
        return
      }
      playRaf = requestAnimationFrame(step)
    }
    playRaf = requestAnimationFrame(step)
  }

  // Stop if the soul or view changes underneath the replay.
  $effect(() => {
    void meta
    void view
    stopPlay()
    playhead = null
  })
  $effect(() => {
    if (r && centreOn !== null) {
      camera = r.centreOn(centreOn, camera)
      invalidate()
    }
  })

  $effect(() => {
    // Refit, not just redraw. With the view locked this is the only thing
    // keeping the graph framed — nobody can pan it back after a resize
    // changes the aspect ratio.
    const onResize = () => {
      needsFit = true
      invalidate()
    }
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      if (raf) cancelAnimationFrame(raf)
    }
  })

  // The view is LOCKED: no pan, no zoom.
  //
  // The layout already fits the window, and at that scale the whole soul is
  // legible without moving anything — so pan and zoom were two ways to leave
  // a good view and no way to get back to it except a button. Rob: "I don't
  // really need to zoom." Removing them also removes every bug they carried:
  // the drag-that-is-really-a-click, the pick radius that had to be divided
  // by the current scale, and a camera that could be left somewhere the
  // reset button was the only escape from.
  //
  // What stays is selection and hover, which is what the stage is actually
  // for. `camera` is now written in exactly one place: the fit.

  function onUp(e: PointerEvent) {
    if (!r) return
    const [lx, ly] = r.toLayout(e.clientX, e.clientY, camera)
    onselect(r.pick(lx, ly, PICK_RADIUS / camera.scale))
  }
  function onMove(e: PointerEvent) {
    if (!r || !meta) return
    const [lx, ly] = r.toLayout(e.clientX, e.clientY, camera)
    const i = r.pick(lx, ly, PICK_RADIUS / camera.scale)
    hover = i !== null ? { doc: meta.docs[i], x: e.clientX, y: e.clientY } : null
  }

  const short = (u: string) => u.split(/[/#]/).pop() || u

  const shown = $derived(
    states ? states.reduce((a, s) => a + (s === NodeState.Hidden ? 0 : 1), 0) : 0,
  )
</script>

<main class="stage">
  <div class="bar">
    {#if meta}
      <span class="figure"><b>{shown}</b> of {meta.node_count} nodes</span>
      <span class="figure"><b>{timeFiltered ? timeFiltered.length / 2 : meta.edge_count}</b> edges</span>
      {#if view === 'spiral'}
        <span class="axis">
          {(meta.turn_dates[0] ?? '').slice(0, 10)} → {(meta.turn_dates[meta.turn_dates.length - 1] ?? '').slice(0, 10)}
          · {meta.turns} turns · angle is time
        </span>
      {:else if selected !== null}
        <span class="axis">radius is hops from <b>{meta.docs[selected].label}</b></span>
      {/if}
    {/if}
    <span class="spacer"></span>
    {#if linkRange}
      <button
        class="play"
        onclick={togglePlay}
        title={playing
          ? 'stop and show every link'
          : 'reveal links in the order they were created, read from the commit each was first asserted in'}
      >{playing ? '\u25a0 stop' : '\u25b6 play links'}</button>
      {#if playing && playhead !== null}
        <span class="playhead">commit {playhead} of {LAST}</span>
      {/if}
    {/if}
    <label><input type="checkbox" bind:checked={showEdges} onchange={invalidate} /> edges</label>
  </div>

  <!-- The store is built by `git lex sync`, not by `git lex save`. A soul can
       be several documents ahead of the graph drawn from it, and every number
       on screen will be internally consistent and wrong about today. That is
       the one thing this view must never let pass quietly, so it sits above
       the graph rather than in a footer. -->
  {#if meta && meta.store_head && meta.store_head !== meta.head_sha}
    <div class="stale">
      <b>This graph is behind the repo.</b>
      The store was last built from commit <code>{meta.store_head.slice(0, 8)}</code>;
      the repo is at <code>{meta.head_sha.slice(0, 8)}</code>
      {#if meta.commits_behind !== null}
        — <b>{meta.commits_behind}</b>
        {meta.commits_behind === 1 ? 'commit' : 'commits'} not in this picture.
      {:else}
        — by an unknown number of commits.
      {/if}
      Run <code>git lex sync</code> in the repo, then reload.
    </div>
  {/if}

  <div class="canvas-wrap">
    {#if err}
      <p class="err">{err}</p>
    {:else if !meta}
      <p class="empty">choose a soul</p>
    {/if}
    <canvas
      bind:this={canvas}
      class:hidden={!meta}
      onpointerup={onUp}
      onpointermove={onMove}
      onpointerleave={() => (hover = null)}
    ></canvas>

    {#if docOpen && meta && selected !== null}
      <DocPanel
        title={meta.docs[selected].label}
        file={docFile}
        loading={docLoading}
        onclose={ondocclose}
      />
    {/if}

    {#if hover && meta}
      <div class="tip" style="left:{hover.x + 14}px; top:{hover.y + 14}px">
        <strong>{hover.doc.label}</strong>
        <span>{meta.classes[hover.doc.class]?.name} · {hover.doc.events} changes</span>
      </div>
    {/if}
  </div>

  {#if meta && (meta.dropped.length || meta.undated)}
    <div class="disclose">
      {#if meta.undated}<span><b>{meta.undated}</b> undated, drawn on the rim</span>{/if}
      {#if meta.links_undated}
        <span title="The store records no commit for these links, so they cannot be placed on the replay timeline. They appear when the replay finishes.">
          <b>{meta.links_undated}</b> links with no recorded birthday
        </span>
      {/if}
      {#each meta.dropped as d}
        <span>
          <b>{d.count}</b> not drawn — {d.reason}
          {#if d.by_predicate?.length}
            <span class="pred">({d.by_predicate
              .slice(0, 3)
              .map(([p, n]) => `${n} ${short(p)}`)
              .join(', ')}{d.by_predicate.length > 3 ? `, +${d.by_predicate.length - 3} more` : ''})</span>
          {/if}
        </span>
      {/each}
    </div>
  {/if}
</main>

<style>
  .stage {
    grid-area: stage;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .bar {
    display: flex; align-items: center; gap: 0.9rem;
    padding: 0.35rem 0.7rem;
    border-bottom: 1px solid var(--rule);
    font-size: 11px; color: var(--ink-soft);
    flex: none;
  }
  .spacer { flex: 1; }
  .figure b { font-variant-numeric: tabular-nums; color: var(--ink); }
  .axis { color: var(--ink-faint); }
  .bar label { display: flex; align-items: center; gap: 0.25rem; }
  .bar button { font-size: 11px; padding: 0.1rem 0.45rem; }

  .stale {
    flex: none;
    background: #fff8e6;
    border-bottom: 1px solid #e8d9a8;
    color: var(--warn);
    padding: 0.35rem 0.7rem;
    font-size: 11px;
  }
  .stale b { color: var(--ink); }
  .stale code { font-size: 10px; }

  .play {
    font-size: 11px;
    padding: 0.1rem 0.45rem;
    border: 1px solid var(--rule);
    background: none;
    cursor: pointer;
  }
  .play:hover { border-color: var(--ink); }
  .playhead {
    font-size: 10px;
    color: var(--ink-faint);
    font-variant-numeric: tabular-nums;
  }

  .canvas-wrap { position: relative; flex: 1; min-height: 0; }
  canvas { width: 100%; height: 100%; display: block; cursor: crosshair; touch-action: none; }
  canvas.hidden { visibility: hidden; }

  .tip {
    position: fixed; z-index: 30; pointer-events: none;
    background: var(--paper); border: 1px solid var(--ink);
    padding: 0.3rem 0.5rem; max-width: 24rem;
    display: flex; flex-direction: column;
  }
  .tip strong { font-family: var(--display); font-weight: 400; font-size: 0.95rem; }
  .tip span { font-size: 10px; color: var(--ink-faint); }

  .disclose {
    flex: none; display: flex; flex-wrap: wrap; gap: 1rem;
    padding: 0.3rem 0.7rem; border-top: 1px solid var(--rule);
    font-size: 10px; color: var(--ink-faint);
  }
  .disclose b { color: var(--ink-soft); }
  .pred { color: var(--ink-faint); }

  .empty, .err {
    position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;
    font-size: 12px; color: var(--ink-faint); margin: 0;
  }
  .err { color: var(--dead); }
</style>
