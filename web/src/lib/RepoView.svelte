<script lang="ts">
  import type { RepoProbe, ServerStatus } from './types'
  import { sparql } from './api'
  import * as Q from './queries'
  import { kitLabel, shortSha, ago } from './format'
  import Spiral from './Spiral.svelte'

  interface Props {
    repo: RepoProbe
    status: ServerStatus | undefined
    onback: () => void
  }
  let { repo, status, onback }: Props = $props()

  type ClassRow = { uri: string; short: string; n: number; titles: string[] }

  let classes = $state<ClassRow[]>([])
  // Counts come from the layout, not from a second query of its own. The
  // census and the picture have to be answering the same question: run
  // separately they disagreed on screen — "File 68" in the legend against
  // "File 135" in the table, both correct, nothing saying why.
  let census = $state<{ file_subjects: number; folded_files: number; unbridged_things: number } | null>(null)
  let planes = $state<{
    fileLinks: number
    fileLinksResolved: number
    things: number
    bridged: number
    thingEdges: number
    selfRefs: number
    literalRefs: { p: string; n: number }[]
  } | null>(null)
  let loading = $state(true)
  let err = $state<string | null>(null)

  const genesis = $derived(repo.genesis_sha ?? '')
  const num = (rows: { n: string }[]) => Number(rows[0]?.n ?? 0)
  const short = (uri: string) => uri.split(/[/#]/).pop() ?? uri

  async function load() {
    if (!genesis) {
      err = 'This repo has no commits, so it has no identity to address a server by.'
      loading = false
      return
    }
    loading = true
    err = null
    try {
      const layoutMeta = fetch(`/api/layout/${genesis}`).then((r) =>
        r.ok ? r.json() : null,
      )
      const [lm, fl, flr, th, br, te, sr, lit] = await Promise.all([
        layoutMeta,
        sparql<{ n: string }>(genesis, Q.fileLinkCount),
        sparql<{ n: string }>(genesis, Q.fileLinkResolved),
        sparql<{ n: string }>(genesis, Q.thingCount),
        sparql<{ n: string }>(genesis, Q.bridged),
        sparql<{ n: string }>(genesis, Q.thingEdges),
        sparql<{ n: string }>(genesis, Q.thingSelfRefs),
        sparql<{ p: string; n: string }>(genesis, Q.thingLiteralRefs),
      ])

      planes = {
        fileLinks: num(fl),
        fileLinksResolved: num(flr),
        things: num(th),
        bridged: num(br),
        thingEdges: num(te),
        selfRefs: num(sr),
        literalRefs: lit.map((r) => ({ p: short(r.p), n: Number(r.n) })),
      }

      census = lm
        ? {
            file_subjects: lm.file_subjects,
            folded_files: lm.folded_files,
            unbridged_things: lm.unbridged_things,
          }
        : null

      // Titles are fetched per class, through the both-planes query. Slower
      // than one big query, but it keeps each class's sample honest rather
      // than letting one chatty class fill the sample for all of them.
      const src: { uri: string; name: string; count: number }[] = lm
        ? lm.classes
        : (await sparql<{ t: string; n: string }>(genesis, Q.classCounts)).map((c) => ({
            uri: c.t,
            name: short(c.t),
            count: Number(c.n),
          }))

      classes = await Promise.all(
        src.map(async (c) => {
          let titles: string[] = []
          try {
            const t = await sparql<{ label: string }>(genesis, Q.titlesFor(c.uri))
            titles = t.map((x) => x.label).filter(Boolean)
          } catch {
            /* a class with no reachable titles is a real answer, not a failure */
          }
          return { uri: c.uri, short: c.name, n: c.count, titles }
        }),
      )
    } catch (e) {
      err = e instanceof Error ? e.message : String(e)
    } finally {
      loading = false
    }
  }

  $effect(() => {
    void genesis
    load()
  })

  const dangling = $derived(planes ? planes.fileLinks - planes.fileLinksResolved : 0)
  const literalTotal = $derived(planes ? planes.literalRefs.reduce((a, b) => a + b.n, 0) : 0)
</script>

<div class="wrap">
  <header>
    <button class="back" onclick={onback}>← all repos</button>
    <h1>{repo.name}</h1>
    <dl class="meta">
      <div><dt>kit</dt><dd title={repo.kit ?? ''}>{kitLabel(repo.kit)}</dd></div>
      <div><dt>commits</dt><dd>{repo.commit_count ?? '—'}</dd></div>
      <div><dt>last commit</dt><dd>{ago(repo.head_time)}</dd></div>
      <div><dt>identity</dt><dd title="genesis sha — the repo's permanent name, and what the server on the other end of every query below was checked against">{shortSha(repo.genesis_sha, 10)}</dd></div>
    </dl>
    <p class="path">{repo.path}</p>

    {#if status}
      <p class="server" class:bad={status.state !== 'ready'}>
        {#if status.state === 'ready'}
          served on port {status.port}
          {#if status.www_dir}
            · frontend from <code>{status.www_dir}</code>
          {/if}
        {:else}
          {status.state}{status.message ? ` — ${status.message}` : ''}
        {/if}
      </p>
    {/if}
  </header>

  {#if err}
    <p class="err">{err}</p>
  {:else if loading}
    <p class="loading">reading the store…</p>
  {:else if planes}
    <section>
      <h2>The two planes</h2>
      <p class="lede">
        A <strong>File</strong> is words at a location at a time. A
        <strong>Thing</strong> is what persists and expresses itself through
        one. Both are real, and they are one join apart — so most of what a
        soul actually wrote is invisible to a query that only looks at Things.
      </p>

      <div class="figures">
        <div class="fig">
          <span class="n">{planes.fileLinks}</span>
          <span class="cap">links written in document bodies</span>
        </div>
        <div class="fig">
          <span class="n">{planes.thingEdges}</span>
          <span class="cap">declared references that are actually edges</span>
        </div>
        <div class="fig">
          <span class="n">{planes.bridged}<span class="of">/{planes.things}</span></span>
          <span class="cap">Things bridged to a file</span>
        </div>
      </div>

      <p class="finding">
        {#if planes.thingEdges === 0 && planes.fileLinks > 0}
          Every one of this soul's <strong>{planes.fileLinks}</strong> connections
          lives on the File plane. The Thing plane has <strong>no edges at
          all</strong>{#if literalTotal > 0}, though {literalTotal} declared
          {literalTotal === 1 ? 'reference names' : 'references name'} something
          in text ({planes.literalRefs.map((r) => `${r.n} ${r.p}`).join(', ')}) —
          named, but not linked, so nothing joins up{/if}. A view that reads only
          declared references would show this soul as empty.
        {:else}
          <strong>{planes.fileLinks}</strong> connections live in document bodies
          against <strong>{planes.thingEdges}</strong> declared between Things.
          The gap is the finding: it is what the soul wrote, as against what it
          was asked to declare.
        {/if}
      </p>

      {#if planes.selfRefs > 0}
        <p class="note">
          A further <strong>{planes.selfRefs}</strong>
          {planes.selfRefs === 1 ? 'triple joins a Thing' : 'triples join Things'}
          to {planes.selfRefs === 1 ? 'itself' : 'themselves'} — every Thing
          carries its own identifier as a property. They are excluded above.
          Counted as connections they would report this soul as
          {planes.thingEdges + planes.selfRefs} times connected on the Thing
          plane instead of {planes.thingEdges}.
        </p>
      {/if}

      {#if dangling > 0}
        <p class="note">
          <strong>{dangling}</strong> of those body links point at a document
          that is not in the store. That is history, not an error — a link to a
          deleted document is often the only surviving evidence the target ever
          existed. Counted here rather than dropped.
        </p>
      {/if}
    </section>

    <section>
      <h2>The whole soul</h2>
      <Spiral {genesis} />
    </section>

    <section>
      <h2>What is in it</h2>
      <div class="scroll-x">
        <table>
          <thead>
            <tr><th>class</th><th class="r">count</th><th>names</th></tr>
          </thead>
          <tbody>
            {#each classes as c (c.uri)}
              <tr>
                <td class="cls">{c.short}</td>
                <td class="r">{c.n}</td>
                <td class="titles">
                  {#if c.titles.length}
                    {c.titles.join(' · ')}
                  {:else}
                    <span class="none">no titles reachable from either plane</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if census}
        <p class="note">
          These are <em>documents</em>, after a Thing and the file it speaks
          through have been folded into one. The store holds
          <strong>{census.file_subjects}</strong> files;
          <strong>{census.folded_files}</strong> of them carry a Thing and are
          counted under that Thing's class, which is why "File only" is the
          smaller number and not a contradiction.
          {#if census.unbridged_things > 0}
            <strong>{census.unbridged_things}</strong>
            {census.unbridged_things === 1 ? 'Thing names a file' : 'Things name files'}
            that {census.unbridged_things === 1 ? 'is' : 'are'} not in the node
            set; {census.unbridged_things === 1 ? 'it is' : 'they are'} drawn
            under {census.unbridged_things === 1 ? 'its' : 'their'} own class.
          {/if}
        </p>
      {/if}
      <p class="note">
        Names are resolved across both planes. A document whose frontmatter says
        <code>title:</code> rather than <code>soul.Note.title:</code> carries
        that title on its <em>file</em>, never on its Thing — so a Thing-plane
        query returns nothing and the document renders as its identifier. That
        is why souls have shown as hex strings.
      </p>
    </section>
  {/if}
</div>

<style>
  .wrap { max-width: 62rem; margin: 0 auto; padding: 2rem 1.5rem 5rem; }

  .back { border: none; padding: 0; background: none; text-decoration: underline; color: var(--ink-soft); font-size: 12px; }
  .back:hover { background: none; color: var(--ink); }

  header { border-bottom: 2px solid var(--rule-strong); padding-bottom: 0.9rem; margin-bottom: 2rem; }
  h1 { font-size: 2.6rem; letter-spacing: -0.02em; margin-top: 0.5rem; line-height: 1; }
  h2 { font-size: 1.35rem; margin-bottom: 0.5rem; }

  .meta { display: flex; flex-wrap: wrap; gap: 1.6rem; margin: 0.9rem 0 0; }
  .meta div { display: flex; flex-direction: column; }
  dt { font-size: 10px; letter-spacing: 0.09em; text-transform: uppercase; color: var(--ink-faint); }
  dd { margin: 0; font-size: 13px; }

  .path { color: var(--ink-faint); font-size: 11px; margin: 0.6rem 0 0; word-break: break-all; }
  .server { font-size: 11px; color: var(--live); margin: 0.35rem 0 0; }
  .server.bad { color: var(--dead); }

  section { margin-bottom: 2.6rem; }
  .lede { color: var(--ink-soft); max-width: 44rem; font-size: 13px; }

  .figures { display: flex; flex-wrap: wrap; gap: 2.4rem; margin: 1.4rem 0; }
  .fig { display: flex; flex-direction: column; }
  .n { font-family: var(--display); font-size: 2.6rem; line-height: 1; font-variant-numeric: tabular-nums; }
  .of { font-size: 1.3rem; color: var(--ink-faint); }
  .cap { font-size: 11px; color: var(--ink-faint); max-width: 13rem; margin-top: 0.3rem; }

  .finding { max-width: 44rem; border-left: 2px solid var(--ink); padding-left: 0.9rem; font-size: 13px; }
  .note { max-width: 44rem; font-size: 11px; color: var(--ink-soft); }

  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th { text-align: left; font-weight: 400; font-size: 10px; letter-spacing: 0.09em; text-transform: uppercase; color: var(--ink-faint); border-bottom: 1px solid var(--rule); padding: 0 0.6rem 0.4rem 0; }
  td { padding: 0.5rem 0.6rem 0.5rem 0; border-bottom: 1px solid var(--rule); vertical-align: top; }
  .r { text-align: right; font-variant-numeric: tabular-nums; }
  .cls { font-family: var(--display); font-size: 1rem; white-space: nowrap; }
  .titles { color: var(--ink-soft); font-size: 12px; }
  .none { color: var(--ink-faint); font-style: italic; }

  .err { color: var(--dead); }
  .loading { color: var(--ink-faint); }
  code { font-size: 11px; }
</style>
