<script lang="ts">
  import { SpiralRenderer, type View, type LayoutOffsets } from './renderer'
  import { ago } from './format'

  interface ClassInfo { uri: string; name: string; count: number; color: string }
  interface DocMeta { id: string; label: string; class: number; born: number | null; events: number }
  interface Dropped { reason: string; count: number; examples: string[] }
  interface LayoutMeta {
    genesis_sha: string
    head_sha: string
    built_at_ms: number
    node_count: number
    edge_count: number
    turns: number
    classes: ClassInfo[]
    docs: DocMeta[]
    turn_dates: (string | null)[]
    undated: number
    first_ordinal: number | null
    last_ordinal: number | null
    dropped: Dropped[]
    offsets: LayoutOffsets
  }

  interface Props { genesis: string }
  let { genesis }: Props = $props()

  let canvas = $state<HTMLCanvasElement | null>(null)
  let meta = $state<LayoutMeta | null>(null)
  let source = $state<string>('')
  let err = $state<string | null>(null)
  let loading = $state(true)
  let showEdges = $state(true)
  let hover = $state<{ doc: DocMeta; x: number; y: number } | null>(null)

  let renderer: SpiralRenderer | null = null
  let view: View = { scale: 1, x: 0, y: 0 }
  let dpr = 1
  let raf = 0

  function frame() {
    if (!renderer) return
    renderer.resize(dpr)
    renderer.draw(view, dpr, showEdges)
    raf = 0
  }
  function invalidate() {
    if (!raf) raf = requestAnimationFrame(frame)
  }

  async function load() {
    loading = true
    err = null
    renderer = null
    meta = null
    try {
      const mr = await fetch(`/api/layout/${genesis}`)
      if (!mr.ok) throw new Error(await mr.text())
      source = mr.headers.get('x-layout-source') ?? ''
      const m: LayoutMeta = await mr.json()

      const dr = await fetch(`/api/layout/${genesis}/data`)
      if (!dr.ok) throw new Error(await dr.text())
      const buf = await dr.arrayBuffer()

      meta = m
      // Wait for the canvas to exist in the DOM before touching WebGL.
      await Promise.resolve()
      if (!canvas) throw new Error('canvas was not mounted')
      dpr = Math.min(window.devicePixelRatio || 1, 2)
      renderer = new SpiralRenderer(canvas, buf, m.offsets, m.node_count, m.turns)
      view = { scale: 0.92, x: 0, y: 0 }
      invalidate()
    } catch (e) {
      err = e instanceof Error ? e.message : String(e)
    } finally {
      loading = false
    }
  }

  $effect(() => {
    void genesis
    load()
    const onResize = () => invalidate()
    window.addEventListener('resize', onResize)
    return () => {
      window.removeEventListener('resize', onResize)
      if (raf) cancelAnimationFrame(raf)
    }
  })

  let drag: { px: number; py: number; ox: number; oy: number } | null = null

  function onDown(e: PointerEvent) {
    if (!canvas) return
    canvas.setPointerCapture(e.pointerId)
    drag = { px: e.clientX, py: e.clientY, ox: view.x, oy: view.y }
  }
  function onUp(e: PointerEvent) {
    canvas?.releasePointerCapture(e.pointerId)
    drag = null
  }
  function onMove(e: PointerEvent) {
    if (!renderer || !canvas) return
    if (drag) {
      const rect = canvas.getBoundingClientRect()
      const aspect = rect.width / rect.height
      view = {
        ...view,
        x: drag.ox + ((e.clientX - drag.px) / rect.width) * 2 * aspect / view.scale,
        y: drag.oy - ((e.clientY - drag.py) / rect.height) * 2 / view.scale,
      }
      hover = null
      invalidate()
      return
    }
    const [lx, ly] = renderer.toLayout(e.clientX, e.clientY, view)
    // Pick radius in layout units, so the target stays a constant size on
    // screen rather than shrinking as you zoom in.
    const i = renderer.pick(lx, ly, 0.02 / view.scale)
    const doc = i != null && meta ? meta.docs[i] : null
    hover = doc ? { doc, x: e.clientX, y: e.clientY } : null
  }
  function onWheel(e: WheelEvent) {
    if (!renderer || !canvas) return
    e.preventDefault()
    const [bx, by] = renderer.toLayout(e.clientX, e.clientY, view)
    const next = Math.min(60, Math.max(0.3, view.scale * Math.exp(-e.deltaY * 0.0016)))
    // Zoom toward the cursor: the point under the pointer stays put.
    const rect = canvas.getBoundingClientRect()
    const aspect = rect.width / rect.height
    const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1
    const ndcY = 1 - ((e.clientY - rect.top) / rect.height) * 2
    view = { scale: next, x: (ndcX * aspect) / next - bx, y: ndcY / next - by }
    invalidate()
  }

  function reset() {
    view = { scale: 0.92, x: 0, y: 0 }
    invalidate()
  }

  const droppedTotal = $derived(meta ? meta.dropped.reduce((a, d) => a + d.count, 0) : 0)
</script>

<div class="spiral">
  {#if err}
    <p class="err">{err}</p>
  {:else if loading}
    <p class="loading">laying out the spiral…</p>
  {/if}

  {#if meta}
    <div class="stage">
      <canvas
        bind:this={canvas}
        onpointerdown={onDown}
        onpointerup={onUp}
        onpointermove={onMove}
        onpointerleave={() => (hover = null)}
        onwheel={onWheel}
      ></canvas>

      {#if hover}
        <div class="tip" style="left:{hover.x + 14}px; top:{hover.y + 14}px">
          <strong>{hover.doc.label}</strong>
          <span class="tclass">{meta.classes[hover.doc.class]?.name}</span>
          <span class="tmeta">
            {hover.doc.events} {hover.doc.events === 1 ? 'change' : 'changes'}
            {#if hover.doc.born !== null} · born at commit {hover.doc.born}{/if}
          </span>
        </div>
      {/if}

      <div class="legend">
        {#each meta.classes as c (c.uri)}
          <div class="lrow">
            <span class="swatch" style="background:{c.color}"></span>
            <span class="lname">{c.name}</span>
            <span class="lcount">{c.count}</span>
          </div>
        {/each}
      </div>

      <div class="controls">
        <label><input type="checkbox" bind:checked={showEdges} onchange={invalidate} /> links</label>
        <button onclick={reset}>reset view</button>
      </div>
    </div>

    <div class="axis">
      <span class="ax">centre {(meta.turn_dates[0] ?? '').slice(0, 10) || 'oldest document'}</span>
      <span class="axline"></span>
      <span class="ax">rim {(meta.turn_dates[meta.turn_dates.length - 1] ?? '').slice(0, 10) || 'newest'}</span>
    </div>

    <p class="reading">
      Angle is <strong>time</strong>: the centre is the
      <strong>oldest document still in the store</strong>, the rim is the
      newest, and one turn of {meta.turns} is one slice of that span. Colour is
      class, and a dot's size is how many times that document has changed.
      {#if meta.first_ordinal !== null && meta.first_ordinal > 1}
        Note that this is not the whole repository: {meta.first_ordinal - 1}
        {meta.first_ordinal - 1 === 1 ? 'commit precedes' : 'commits precede'}
        the centre, having left no document behind that still exists.
      {/if}
    </p>

    <p class="caveat">
      A document sits where it was <strong>saved</strong>, not where it was
      written — measured across these repos the gap is usually about three
      minutes, but one document in six lands in the wrong turn and the worst
      case was 65 days. Documents saved in the same commit are spread
      <em>across</em> the track rather than along it, so an afternoon's import
      reads as a bar and not as three weeks of work.
    </p>

    <p class="stats">
      {meta.node_count} documents · {meta.edge_count} links drawn
      {#if meta.undated > 0}
        · <strong>{meta.undated}</strong> could not be dated and sit on the rim
      {/if}
      {#if droppedTotal > 0}
        · <strong>{droppedTotal}</strong> not drawn
      {/if}
      · layout {source === 'cache' ? `from cache, built ${ago(new Date(meta.built_at_ms).toISOString())}` : 'computed just now'}
      from commit <code>{meta.head_sha.slice(0, 8)}</code>
    </p>

    {#if droppedTotal > 0}
      <ul class="dropped">
        {#each meta.dropped as d}
          <li>
            <strong>{d.count}</strong> {d.reason}
            {#if d.examples.length}
              <div class="ex">{d.examples.slice(0, 3).map((e) => e.split('/').pop()).join(' · ')}</div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .spiral { margin-top: 1rem; }
  .stage { position: relative; width: 100%; aspect-ratio: 1 / 1; max-height: 78vh; border: 1px solid var(--rule); }
  canvas { width: 100%; height: 100%; display: block; cursor: grab; touch-action: none; }
  canvas:active { cursor: grabbing; }

  .tip {
    position: fixed; z-index: 20; pointer-events: none;
    background: var(--paper); border: 1px solid var(--ink);
    padding: 0.35rem 0.55rem; max-width: 22rem;
    display: flex; flex-direction: column; gap: 1px;
  }
  .tip strong { font-family: var(--display); font-size: 0.95rem; font-weight: 400; }
  .tclass { font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--ink-faint); }
  .tmeta { font-size: 10px; color: var(--ink-soft); }

  .legend {
    position: absolute; left: 0.7rem; top: 0.7rem;
    background: rgba(255,255,255,0.9); padding: 0.4rem 0.55rem;
    display: flex; flex-direction: column; gap: 1px;
  }
  .lrow { display: flex; align-items: center; gap: 0.4rem; font-size: 11px; }
  .swatch { width: 9px; height: 9px; display: inline-block; }
  .lname { min-width: 5.5rem; }
  .lcount { color: var(--ink-faint); font-variant-numeric: tabular-nums; }

  .controls {
    position: absolute; right: 0.7rem; top: 0.7rem;
    display: flex; gap: 0.5rem; align-items: center;
    background: rgba(255,255,255,0.9); padding: 0.3rem 0.5rem;
    font-size: 11px;
  }
  .controls label { display: flex; align-items: center; gap: 0.25rem; }

  .axis { display: flex; align-items: center; gap: 0.6rem; margin-top: 0.5rem; }
  .ax { font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--ink-faint); white-space: nowrap; }
  .axline { flex: 1; height: 1px; background: var(--rule); }

  .reading, .caveat, .stats { max-width: 46rem; }
  .reading { font-size: 13px; color: var(--ink-soft); margin-top: 0.9rem; }
  .caveat { font-size: 11px; color: var(--ink-soft); border-left: 2px solid var(--rule); padding-left: 0.7rem; }
  .stats { font-size: 11px; color: var(--ink-faint); }
  .dropped { list-style: none; padding: 0; margin: 0.3rem 0 0; font-size: 11px; color: var(--ink-soft); max-width: 46rem; }
  .dropped li { margin-bottom: 0.25rem; }
  .ex { color: var(--ink-faint); font-size: 10px; }

  .err { color: var(--dead); }
  .loading { color: var(--ink-faint); }
  code { font-size: 11px; }
</style>
