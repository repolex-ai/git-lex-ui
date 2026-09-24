<script lang="ts">
  import type { LayoutMeta, DocMeta, Adjacency } from './graph'
  import { triplesFor, curie, shortUri, type Triple } from './triples'
  import type { FileText } from './types'
  import DocPanel from './DocPanel.svelte'

  interface Props {
    genesis: string | null
    meta: LayoutMeta | null
    adj: Adjacency | null
    selected: number | null
    onselect: (i: number) => void
    docFile: FileText | null
    docLoading: boolean
  }
  let { genesis, meta, adj, selected, onselect, docFile, docLoading }: Props = $props()

  // Two tabs in one fixed place: the document you are reading, and what the
  // graph knows about it. Which one is showing survives clicking from node to
  // node, so following links while reading keeps you reading. It is
  // remembered per browser as a convenience; losing it costs one click.
  type Tab = 'document' | 'info'
  const TAB_KEY = 'git-lex-ui.inspector-tab'
  function savedTab(): Tab {
    try {
      return localStorage.getItem(TAB_KEY) === 'info' ? 'info' : 'document'
    } catch {
      return 'document'
    }
  }
  let tab = $state<Tab>(savedTab())
  function setTab(t: Tab) {
    tab = t
    try { localStorage.setItem(TAB_KEY, t) } catch { /* private window */ }
  }

  let rows = $state<{
    subjects: string[]
    asSubject: Triple[]
    asObject: Triple[]
    twinOf: Map<string, string>
  } | null>(null)
  let loading = $state(false)
  let err = $state<string | null>(null)

  const doc = $derived<DocMeta | null>(
    meta && selected !== null ? (meta.docs[selected] ?? null) : null,
  )
  const cls = $derived(doc && meta ? meta.classes[doc.class] : null)

  // Index by document id so a triple's object can be turned back into a node
  // on the stage — this is what makes the panel a way to navigate rather than
  // just a way to read.
  const indexById = $derived(
    meta ? new Map(meta.docs.map((d, i) => [d.id, i])) : new Map<string, number>(),
  )

  $effect(() => {
    const g = genesis
    const d = doc
    if (!g || !d) {
      rows = null
      return
    }
    let cancelled = false
    loading = true
    err = null
    triplesFor(g, d.id)
      .then((r) => { if (!cancelled) rows = r })
      .catch((e) => { if (!cancelled) err = e instanceof Error ? e.message : String(e) })
      .finally(() => { if (!cancelled) loading = false })
    return () => { cancelled = true }
  })

  /** Resolve a URI to a node on the stage, following the Thing -> File fold
   *  when the URI names a Thing. */
  function nodeFor(uri: string): number | undefined {
    const direct = indexById.get(uri)
    if (direct !== undefined) return direct
    const file = rows?.twinOf.get(uri)
    return file ? indexById.get(file) : undefined
  }

  function jump(uri: string) {
    const i = nodeFor(uri)
    if (i !== undefined) onselect(i)
  }

  const outLinks = $derived(
    doc && adj && selected !== null && meta
      ? adj.out[selected].map((j) => ({ i: j, d: meta.docs[j] }))
      : [],
  )
  const inLinks = $derived(
    doc && adj && selected !== null && meta
      ? adj.inc[selected].map((j) => ({ i: j, d: meta.docs[j] }))
      : [],
  )
</script>

<aside class="rail">
  <!--
    The zones are permanent. When nothing is selected this panel says so in
    place rather than collapsing: a control that moves or disappears is a
    control you have to re-find every time the state changes.
  -->
  <section class="block head">
    <h2>selected</h2>
    {#if doc && cls}
      <div class="title">{doc.label}</div>
      <div class="meta">
        <span class="swatch" style="background:{cls.color}"></span>{cls.name}
      </div>
      <dl>
        <div><dt>born at commit</dt><dd>{doc.born ?? 'not dated'}</dd></div>
        <div><dt>changes</dt><dd>{doc.events}</dd></div>
        <div><dt>links</dt><dd>{outLinks.length} out · {inLinks.length} in</dd></div>
      </dl>
      <div class="uri" title={doc.id}>{doc.id}</div>
    {:else}
      <p class="empty">click a node on the stage</p>
    {/if}
  </section>

  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={tab === 'document'} class:on={tab === 'document'} onclick={() => setTab('document')}>document</button>
    <button role="tab" aria-selected={tab === 'info'} class:on={tab === 'info'} onclick={() => setTab('info')}>
      links &amp; triples
      {#if doc}<span class="n">{outLinks.length + inLinks.length}</span>{/if}
    </button>
  </div>

  {#if tab === 'document'}
  <div class="pane">
    {#if doc}
      <DocPanel file={docFile} loading={docLoading} />
    {:else}
      <p class="empty pad">click a node on the stage to read it here</p>
    {/if}
  </div>
  {:else}
  <div class="pane scroll">
  <section class="block links">
    <h2>links out <span class="n">{outLinks.length}</span></h2>
    {#if outLinks.length}
      <ul>
        {#each outLinks as l (l.i)}
          <li><button onclick={() => onselect(l.i)}>→ {l.d.label}</button></li>
        {/each}
      </ul>
    {:else}
      <p class="empty">none</p>
    {/if}

    <h2>links in <span class="n">{inLinks.length}</span></h2>
    {#if inLinks.length}
      <ul>
        {#each inLinks as l (l.i)}
          <li><button onclick={() => onselect(l.i)}>← {l.d.label}</button></li>
        {/each}
      </ul>
    {:else}
      <p class="empty">none</p>
    {/if}
  </section>

  <section class="block triples">
    <h2>
      triples
      {#if rows}<span class="n">{rows.asSubject.length + rows.asObject.length}</span>{/if}
    </h2>
    {#if err}
      <p class="err">{err}</p>
    {:else if loading}
      <p class="empty">reading…</p>
    {:else if rows}
      <!-- Both planes. A document is a File and usually a Thing, and its
           facts are split across the two; showing one is how a document looks
           half-empty when it is not. -->
      {#if rows.subjects.length > 1}
        <p class="planes">{rows.subjects.length} subjects — this document's File and its Thing</p>
      {/if}
      <table>
        <tbody>
          {#each rows.asSubject as t}
            <tr>
              <td class="p" title={t.p}>{curie(t.p)}</td>
              <td class="o">
                {#if t.oIsUri && nodeFor(t.o) !== undefined}
                  <button class="link" onclick={() => jump(t.o)}>{shortUri(t.o)}</button>
                {:else if t.oIsUri}
                  <span class="uriv" title={t.o}>{curie(t.o)}</span>
                {:else}
                  <span class="lit">{t.o}</span>
                {/if}
              </td>
            </tr>
          {/each}
          {#each rows.asObject as t}
            <tr class="inbound">
              <td class="p" title={t.p}>← {curie(t.p)}</td>
              <td class="o">
                {#if nodeFor(t.s) !== undefined}
                  <button class="link" onclick={() => jump(t.s)}>{shortUri(t.s)}</button>
                {:else}
                  <span class="uriv" title={t.s}>{shortUri(t.s)}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="empty">nothing selected</p>
    {/if}
  </section>
  </div>
  {/if}
</aside>

<style>

  .rail {
    grid-area: right;
    border-left: 1px solid var(--rule);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--paper);
  }
  .block { border-bottom: 1px solid var(--rule); padding: 0.6rem 0.7rem; }
  .block:last-child { border-bottom: none; }

  .tabs { display: flex; border-bottom: 1px solid var(--rule); }
  .tabs button {
    flex: 1; border: none; background: none; border-radius: 0;
    padding: 0.4rem 0.5rem; font-family: var(--mono); font-size: 10px;
    letter-spacing: 0.1em; text-transform: uppercase; color: var(--ink-faint);
    border-bottom: 2px solid transparent; margin-bottom: -1px;
    display: flex; gap: 0.4rem; justify-content: center;
  }
  .tabs button:hover { color: var(--ink); background: none; }
  .tabs button.on { color: var(--ink); border-bottom-color: var(--ink); }

  .pane { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .pane.scroll { overflow-y: auto; display: block; }
  .pad { padding: 0.6rem 0.7rem; }

  h2 {
    font-family: var(--mono); font-size: 10px; letter-spacing: 0.12em;
    text-transform: uppercase; color: var(--ink-faint);
    margin: 0 0 0.35rem; display: flex; gap: 0.4rem;
  }
  h2:not(:first-child) { margin-top: 0.8rem; }
  .n { color: var(--ink-faint); }

  .title { font-family: var(--display); font-size: 1.15rem; line-height: 1.2; }
  .meta { display: flex; align-items: center; gap: 0.35rem; font-size: 11px; color: var(--ink-soft); margin-top: 0.15rem; }
  .swatch { width: 9px; height: 9px; display: inline-block; }

  dl { display: flex; flex-wrap: wrap; gap: 0.9rem; margin: 0.5rem 0 0; }
  dl div { display: flex; flex-direction: column; }
  dt { font-size: 9px; letter-spacing: 0.09em; text-transform: uppercase; color: var(--ink-faint); }
  dd { margin: 0; font-size: 12px; font-variant-numeric: tabular-nums; }

  .uri { font-size: 9px; color: var(--ink-faint); margin-top: 0.45rem; word-break: break-all; }

  ul { list-style: none; margin: 0; padding: 0; }
  li button {
    border: none; background: none; padding: 0.1rem 0.2rem;
    font-size: 11px; text-align: left; width: 100%;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  li button:hover { background: var(--paper-tint); color: inherit; }

  table { width: 100%; border-collapse: collapse; }
  td { vertical-align: top; padding: 0.12rem 0.2rem 0.12rem 0; font-size: 10px; border-bottom: 1px solid var(--paper-tint); }
  .p { color: var(--ink-soft); white-space: nowrap; width: 1%; padding-right: 0.5rem; }
  .inbound .p { color: var(--ink-faint); }
  .o { word-break: break-word; }
  .lit { color: var(--ink); }
  .uriv { color: var(--ink-faint); }
  .link { border: none; background: none; padding: 0; font-size: 10px; text-decoration: underline; }
  .link:hover { background: none; color: var(--ink); }

  .planes { font-size: 9px; color: var(--ink-faint); margin: 0 0 0.3rem; }
  .empty { font-size: 11px; color: var(--ink-faint); margin: 0; }
  .err { font-size: 10px; color: var(--dead); margin: 0; }
</style>
