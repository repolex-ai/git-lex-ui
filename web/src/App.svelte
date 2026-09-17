<script lang="ts">
  import { api } from './lib/api'
  import type { ReposResponse, RepoProbe, ServerStatus } from './lib/types'
  import {
    loadLayout, Adjacency, trackPoints, neighbourhoodPositions, edgesToDraw,
    computeStates, type LayoutMeta,
  } from './lib/graph'
  import type { GraphRenderer } from './lib/renderer'
  import LeftRail from './lib/LeftRail.svelte'
  import Stage from './lib/Stage.svelte'
  import Inspector from './lib/Inspector.svelte'
  import type { FileText, SyncState } from './lib/types'

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
  let visiblePredicates = $state<Set<number>>(new Set())
  let search = $state('')
  let selected = $state<number | null>(null)

  // --- the document panel -------------------------------------------------
  //
  // Floats over the stage rather than living in the inspector, matching
  // the old viewer's behaviour, where a document is something you open on top
  // of the graph and dismiss, not a section competing with the triples for
  // room in a 18rem rail.
  let docFile = $state<FileText | null>(null)
  let docLoading = $state(false)
  let docOpen = $state(false)

  /** The repo-relative path a File IRI carries.
   *
   *  `https://repolex.ai/git-lex/File/Soul/Note/x.md` -> `Soul/Note/x.md`.
   *  Null for anything that is not a File IRI — a Thing has no path, because
   *  a Thing is not a file. */
  const FILE_PREFIX = 'https://repolex.ai/git-lex/File/'
  function pathOf(id: string): string | null {
    return id.startsWith(FILE_PREFIX) ? decodeURIComponent(id.slice(FILE_PREFIX.length)) : null
  }

  $effect(() => {
    const g = current?.genesis_sha ?? null
    const d = meta && selected !== null ? meta.docs[selected] : null
    const rel = d ? pathOf(d.id) : null
    if (!g || !rel) {
      docFile = null
      docOpen = false
      return
    }
    let cancelled = false
    docLoading = true
    docOpen = true
    api.file(g, rel).then((f) => {
      if (!cancelled) {
        docFile = f
        docLoading = false
      }
    })
    return () => {
      cancelled = true
    }
  })
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

  // --- sync ---------------------------------------------------------------
  let syncs = $state<Record<string, SyncState>>({})
  let syncTimer: ReturnType<typeof setInterval> | null = null

  async function pollSyncs() {
    try {
      const next = await api.syncs()
      // A sync that just finished changes what the list and the open graph
      // should say, so refresh both when any running entry stops running.
      const finished = Object.entries(syncs).filter(
        ([p, st]) => st.state === 'running' && next[p] && next[p].state !== 'running',
      )
      syncs = next
      if (finished.length) {
        await loadRepos()
        const open = current
        if (open && finished.some(([p]) => p === open.path)) await openRepo(open)
      }
      if (!Object.values(next).some((st) => st.state === 'running') && syncTimer) {
        clearInterval(syncTimer)
        syncTimer = null
      }
    } catch { /* the next tick will say so */ }
  }

  async function startSync(repo: RepoProbe) {
    try {
      const st = await api.sync(repo.path)
      syncs = { ...syncs, [repo.path]: st }
      if (!syncTimer) syncTimer = setInterval(pollSyncs, 2000)
    } catch (e) {
      syncs = {
        ...syncs,
        [repo.path]: { state: 'failed', ms: 0, message: e instanceof Error ? e.message : String(e) },
      }
    }
  }

  $effect(() => {
    loadRepos()
    pollServers()
    pollSyncs()
    const t = setInterval(pollServers, 5000)
    return () => {
      clearInterval(t)
      if (syncTimer) clearInterval(syncTimer)
    }
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
      visiblePredicates = new Set(l.meta.predicates.map((_, i) => i))

      // ?links=linksTo,relatedToId restricts the drawn link kinds, so a
      // filtered reading of a soul is a thing you can send someone rather
      // than a set of clicks you have to describe.
      const wantLinks = new URLSearchParams(location.search).get('links')
      if (wantLinks) {
        const names = new Set(wantLinks.split(',').map((x) => x.trim()).filter(Boolean))
        const picked = l.meta.predicates
          .map((p, i) => [p, i] as const)
          .filter(([p]) => names.has(p.name) || names.has(p.uri))
          .map(([, i]) => i)
        // An unmatched name leaves the filter alone rather than blanking the
        // stage: a typo should not look like a soul with no links.
        if (picked.length) visiblePredicates = new Set(picked)
      }

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
  const allEdges = $derived(
    buffer && meta
      ? new Uint32Array(buffer.slice(meta.offsets.edges, meta.offsets.edges + meta.offsets.edges_bytes))
      : null,
  )
  const edgePredicates = $derived(
    buffer && meta
      ? new Uint16Array(
          buffer.slice(
            meta.offsets.edge_predicates,
            meta.offsets.edge_predicates + meta.offsets.edge_predicates_bytes,
          ),
        )
      : null,
  )
  const edgeSubset = $derived(
    allEdges && edgePredicates && meta
      ? edgesToDraw(
          allEdges,
          edgePredicates,
          visiblePredicates.size === meta.predicates.length ? null : visiblePredicates,
          neighbourhood ? neighbourhood.members : null,
          computed?.states ?? null,
        )
      : null,
  )

  /** A document is addressable, not just reachable by clicking. The soul is
   *  named by its genesis sha and the document by its URI, so a link to
   *  "this exact row in this exact soul" survives being pasted to someone
   *  else. */
  function writeUrl(repo: RepoProbe | null, doc: number | null) {
    if (!repo?.genesis_sha) return
    const q = new URLSearchParams()
    if (doc !== null && meta) q.set('doc', meta.docs[doc].id)
    if (meta && visiblePredicates.size < meta.predicates.length) {
      q.set(
        'links',
        meta.predicates.filter((_, i) => visiblePredicates.has(i)).map((p) => p.name).join(','),
      )
    }
    const qs = q.toString()
    history.replaceState(null, '', `${location.pathname}${qs ? '?' + qs : ''}#${repo.genesis_sha}`)
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
  function togglePredicate(i: number) {
    const next = new Set(visiblePredicates)
    if (next.has(i)) next.delete(i)
    else next.add(i)
    visiblePredicates = next
    writeUrl(current, selected)
  }
  function onlyPredicate(i: number) {
    visiblePredicates = new Set([i])
    writeUrl(current, selected)
  }
  function allPredicates() {
    visiblePredicates = new Set((meta?.predicates ?? []).map((_, i) => i))
    writeUrl(current, selected)
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
    {syncs}
    onsync={startSync}
    ontoggle={toggleClass}
    onlyclass={onlyClass}
    onallclasses={allClasses}
    onsearch={(s) => (search = s)}
    onview={setView}
    {visiblePredicates}
    ontogglepredicate={togglePredicate}
    onlypredicate={onlyPredicate}
    onallpredicates={allPredicates}
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
    {docOpen}
    {docFile}
    {docLoading}
    ondocclose={() => (docOpen = false)}
    syncState={current ? (syncs[current.path] ?? null) : null}
    onsync={() => current && startSync(current)}
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
