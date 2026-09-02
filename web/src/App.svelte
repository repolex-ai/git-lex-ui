<script lang="ts">
  import { api } from './lib/api'
  import type { ReposResponse, RepoProbe, ServerStatus } from './lib/types'
  import {
    loadLayout, Adjacency, trackPoints, neighbourhoodPositions, edgesWithin,
    computeStates, type LayoutMeta,
  } from './lib/graph'
  import type { GraphRenderer } from './lib/renderer'
  import LeftRail from './lib/LeftRail.svelte'
  import Stage from './lib/Stage.svelte'
  import Inspector from './lib/Inspector.svelte'

  let data = $state<ReposResponse | null>(null)
  let servers = $state<Record<string, ServerStatus>>({})
  let error = $state<string | null>(null)
  let busy = $state<string | null>(null)

  let current = $state<RepoProbe | null>(null)
  let meta = $state<LayoutMeta | null>(null)
  let buffer = $state<ArrayBuffer | null>(null)
  let adj = $state<Adjacency | null>(null)
  let spiralPositions = $state<Float32Array | null>(null)
  let layoutErr = $state<string | null>(null)

  let visibleClasses = $state<Set<number>>(new Set())
  let search = $state('')
  let selected = $state<number | null>(null)
  let view = $state<'spiral' | 'neighbourhood'>('spiral')
  let centreOn = $state<number | null>(null)
  let hops = 2

  async function loadRepos() {
    try {
      data = await api.repos()
      error = null
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
  }

  // Health is asked, never remembered: a page left open finds out its server
  // died rather than continuing to render the last data it had.
  async function pollServers() {
    try {
      const list = await api.servers()
      servers = Object.fromEntries(list.map((s) => [s.path, s]))
    } catch { /* the next tick will say so */ }
  }

  $effect(() => {
    loadRepos()
    pollServers()
    const t = setInterval(pollServers, 5000)
    return () => clearInterval(t)
  })

  async function openRepo(repo: RepoProbe) {
    busy = repo.path
    layoutErr = null
    try {
      const st = await api.open(repo.path)
      servers = { ...servers, [repo.path]: st }
      if (st.state !== 'ready') {
        layoutErr = st.message ?? `server is ${st.state}`
        return
      }
      current = repo

      meta = null
      buffer = null
      selected = null
      search = ''
      view = 'spiral'

      const l = await loadLayout(repo.genesis_sha!)
      meta = l.meta
      buffer = l.buffer
      adj = new Adjacency(l.meta.node_count, new Uint32Array(
        l.buffer.slice(l.meta.offsets.edges, l.meta.offsets.edges + l.meta.offsets.edges_bytes),
      ))
      spiralPositions = new Float32Array(
        l.buffer.slice(l.meta.offsets.positions, l.meta.offsets.positions + l.meta.offsets.positions_bytes),
      )
      visibleClasses = new Set(l.meta.classes.map((_, i) => i))

      const want = new URLSearchParams(location.search).get('doc')
      if (want) {
        const i = l.meta.docs.findIndex((d) => d.id === want)
        if (i >= 0) {
          selected = i
          centreOn = i
          queueMicrotask(() => (centreOn = null))
        }
      }
    } catch (e) {
      layoutErr = e instanceof Error ? e.message : String(e)
    } finally {
      busy = null
    }
  }

  // Open whatever the URL names, so a link to a soul is a real address.
  $effect(() => {
    if (!data || current) return
    const want = location.hash.replace(/^#/, '')
    const repo = want ? data.repos.find((r) => r.genesis_sha === want) : null
    if (repo) openRepo(repo)
  })

  const neighbourhood = $derived(
    view === 'neighbourhood' && meta && adj && selected !== null
      ? neighbourhoodPositions(meta.node_count, adj, selected, hops)
      : null,
  )

  const computed = $derived(
    meta
      ? computeStates({
          n: meta.node_count,
          docs: meta.docs,
          visibleClasses,
          search: search.trim().toLowerCase(),
          selected,
          restrictTo: neighbourhood ? neighbourhood.members : null,
        })
      : null,
  )

  const positions = $derived(neighbourhood ? neighbourhood.positions : spiralPositions)
  const track = $derived(view === 'spiral' && meta ? trackPoints(meta.turns) : null)
  const edgeSubset = $derived(
    neighbourhood && adj && buffer && meta
      ? edgesWithin(
          new Uint32Array(buffer.slice(meta.offsets.edges, meta.offsets.edges + meta.offsets.edges_bytes)),
          neighbourhood.members,
        )
      : null,
  )

  /** A document is addressable, not just reachable by clicking. The soul is
   *  named by its genesis sha and the document by its URI, so a link to
   *  "this exact row in this exact soul" survives being pasted to someone
   *  else. */
  function writeUrl(repo: RepoProbe | null, doc: number | null) {
    if (!repo?.genesis_sha) return
    const q = doc !== null && meta ? `?doc=${encodeURIComponent(meta.docs[doc].id)}` : ''
    history.replaceState(null, '', `${location.pathname}${q}#${repo.genesis_sha}`)
  }

  function select(i: number | null) {
    selected = i
    centreOn = null
    writeUrl(current, i)
  }
  function selectAndCentre(i: number) {
    selected = i
    centreOn = i
    writeUrl(current, i)
    // Reset so the same node can be centred on twice in a row.
    queueMicrotask(() => (centreOn = null))
  }
  function toggleClass(i: number) {
    const next = new Set(visibleClasses)
    if (next.has(i)) next.delete(i)
    else next.add(i)
    visibleClasses = next
  }
  function onlyClass(i: number) {
    visibleClasses = new Set([i])
  }
  function allClasses() {
    visibleClasses = new Set((meta?.classes ?? []).map((_, i) => i))
  }
  function setView(v: 'spiral' | 'neighbourhood') {
    view = v
  }
  function onReady(_r: GraphRenderer) { /* renderer owned by the stage */ }
</script>

<div class="app">
  <header class="top">
    <span class="brand">git-lex</span>
    {#if current}
      <span class="here">{current.name}</span>
      <span class="path" title={current.path}>{current.path}</span>
      {#if meta}
        <span class="head">commit {meta.head_sha.slice(0, 8)}</span>
      {/if}
    {:else}
      <span class="path">no soul open</span>
    {/if}
    <span class="spacer"></span>
    {#if data}
      <span class="dropped" title={data.dropped.map((d) => `${d.path} — ${d.reason}`).join('\n')}>
        {data.counts.shown} shown · {data.counts.dropped_total} registry entries dropped
      </span>
    {/if}
  </header>

  {#if error}<div class="err">{error}</div>{/if}
  {#if layoutErr}<div class="err">{layoutErr}</div>{/if}

  <LeftRail
    repos={data?.repos ?? []}
    {servers}
    {current}
    {busy}
    {meta}
    {visibleClasses}
    {search}
    matchCount={computed?.matches.length ?? 0}
    {view}
    hasSelection={selected !== null}
    onpick={openRepo}
    ontoggle={toggleClass}
    onlyclass={onlyClass}
    onallclasses={allClasses}
    onsearch={(s) => (search = s)}
    onview={setView}
  />

  <Stage
    {meta}
    {buffer}
    states={computed?.states ?? null}
    {track}
    {positions}
    {edgeSubset}
    {selected}
    {view}
    {centreOn}
    onselect={select}
    onready={onReady}
  />

  <Inspector
    genesis={current?.genesis_sha ?? null}
    {meta}
    {adj}
    {selected}
    onselect={selectAndCentre}
  />
</div>

<style>
  /* Fixed zones. The rails keep their home and their width; the stage takes
     what is left. Nothing here opens over the graph or moves when state
     changes — a control you have to re-find is a control you stop using. */
  .app {
    display: grid;
    grid-template-columns: 15rem 1fr 18rem;
    grid-template-rows: auto 1fr;
    grid-template-areas:
      "top   top   top"
      "left  stage right";
    height: 100vh;
    overflow: hidden;
  }

  .top {
    grid-area: top;
    display: flex; align-items: baseline; gap: 0.7rem;
    padding: 0.4rem 0.7rem;
    border-bottom: 1px solid var(--rule-strong);
    font-size: 11px; color: var(--ink-faint);
  }
  .brand { font-family: var(--display); font-size: 1.05rem; color: var(--ink); letter-spacing: -0.01em; }
  .here { font-family: var(--display); font-size: 1.05rem; color: var(--ink); }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .head { font-variant-numeric: tabular-nums; }
  .spacer { flex: 1; }
  .dropped { white-space: nowrap; }

  .err {
    grid-column: 1 / -1;
    background: #fdf5f5; color: var(--dead);
    border-bottom: 1px solid #f0d4d4;
    padding: 0.3rem 0.7rem; font-size: 11px;
  }
</style>
