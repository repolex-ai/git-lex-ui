<script lang="ts">
  import { GraphRenderer, NodeState, type View } from './renderer'
  import type { LayoutMeta, DocMeta } from './graph'

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
  }
  let {
    meta, buffer, states, track, positions, edgeSubset,
    selected, view, centreOn, onselect, onready,
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
  $effect(() => {
    if (!r) return
    r.setEdgeSubset(edgeSubset ?? r.edges)
    invalidate()
  })
  $effect(() => {
    if (r && centreOn !== null) {
      camera = r.centreOn(centreOn, camera)
      invalidate()
    }
  })

  $effect(() => {
    const onResize = () => invalidate()
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      if (raf) cancelAnimationFrame(raf)
    }
  })

  let drag: { px: number; py: number; ox: number; oy: number; moved: boolean } | null = null

  function onDown(e: PointerEvent) {
    canvas?.setPointerCapture(e.pointerId)
    drag = { px: e.clientX, py: e.clientY, ox: camera.x, oy: camera.y, moved: false }
  }
  function onUp(e: PointerEvent) {
    canvas?.releasePointerCapture(e.pointerId)
    // A click is a drag that never moved. Without this, every pan that ends
    // over a node also selects it.
    if (drag && !drag.moved && r) {
      const [lx, ly] = r.toLayout(e.clientX, e.clientY, camera)
      onselect(r.pick(lx, ly, 0.02 / camera.scale))
    }
    drag = null
  }
  function onMove(e: PointerEvent) {
    if (!r || !canvas) return
    if (drag) {
      const rect = canvas.getBoundingClientRect()
      const aspect = rect.width / rect.height
      if (Math.abs(e.clientX - drag.px) + Math.abs(e.clientY - drag.py) > 3) drag.moved = true
      camera = {
        ...camera,
        x: drag.ox + ((e.clientX - drag.px) / rect.width) * 2 * aspect / camera.scale,
        y: drag.oy - ((e.clientY - drag.py) / rect.height) * 2 / camera.scale,
      }
      hover = null
      invalidate()
      return
    }
    const [lx, ly] = r.toLayout(e.clientX, e.clientY, camera)
    const i = r.pick(lx, ly, 0.02 / camera.scale)
    hover = i !== null && meta ? { doc: meta.docs[i], x: e.clientX, y: e.clientY } : null
  }
  function onWheel(e: WheelEvent) {
    if (!r || !canvas) return
    e.preventDefault()
    const [bx, by] = r.toLayout(e.clientX, e.clientY, camera)
    const next = Math.min(80, Math.max(0.25, camera.scale * Math.exp(-e.deltaY * 0.0016)))
    const rect = canvas.getBoundingClientRect()
    const aspect = rect.width / rect.height
    const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1
    const ndcY = 1 - ((e.clientY - rect.top) / rect.height) * 2
    camera = { scale: next, x: (ndcX * aspect) / next - bx, y: ndcY / next - by }
    invalidate()
  }
  function reset() {
    camera = r && canvas
      ? r.fitView(canvas.clientWidth / Math.max(1, canvas.clientHeight))
      : { scale: 0.92, x: 0, y: 0 }
    invalidate()
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
      <span class="figure"><b>{edgeSubset ? edgeSubset.length / 2 : meta.edge_count}</b> edges</span>
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
    <label><input type="checkbox" bind:checked={showEdges} onchange={invalidate} /> edges</label>
    <button onclick={reset}>reset view</button>
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
      onpointerdown={onDown}
      onpointerup={onUp}
      onpointermove={onMove}
      onpointerleave={() => (hover = null)}
      onwheel={onWheel}
    ></canvas>

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
