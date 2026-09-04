<script lang="ts">
  import type { RepoProbe, ServerStatus } from './types'
  import type { LayoutMeta } from './graph'
  import { kitLabel, ago } from './format'

  interface Props {
    repos: RepoProbe[]
    servers: Record<string, ServerStatus>
    current: RepoProbe | null
    busy: string | null
    meta: LayoutMeta | null
    visibleClasses: Set<number>
    search: string
    matchCount: number
    view: 'spiral' | 'neighbourhood'
    hasSelection: boolean
    onpick: (r: RepoProbe) => void
    ontoggle: (i: number) => void
    onlyclass: (i: number) => void
    onallclasses: () => void
    onsearch: (s: string) => void
    onview: (v: 'spiral' | 'neighbourhood') => void
    visiblePredicates: Set<number>
    ontogglepredicate: (i: number) => void
    onlypredicate: (i: number) => void
    onallpredicates: () => void
  }
  let {
    repos, servers, current, busy, meta, visibleClasses, search, matchCount,
    view, hasSelection, onpick, ontoggle, onlyclass, onallclasses, onsearch, onview,
    visiblePredicates, ontogglepredicate, onlypredicate, onallpredicates,
  }: Props = $props()

  /** The staleness marker for one row: what to draw, and what it means.
   *
   *  Measured across the 24 registered souls on 2026-09-03: 13 of the 15 that
   *  have ever been synced were behind, 74 commits in total. So the common
   *  case is "behind", and the marker has to be legible at a glance without
   *  shouting — this is a normal condition, not an error.
   *
   *  `unplaceable` and `never-synced` get their own glyph rather than being
   *  folded into "current". They mean the question was unanswerable from
   *  disk, and rendering unanswerable as fine is the exact defect this file
   *  spends its comments warning about. */
  function stale(r: RepoProbe): { mark: string; cls: string; title: string } | null {
    const g = r.graph
    if (!g) return null
    switch (g.state) {
      case 'current':
        return null
      case 'behind': {
        const n = g.commits
        return {
          mark: n === null ? '\u21ba?' : `\u21ba${n}`,
          cls: 'behind',
          title:
            n === null
              ? `the graph was built at ${g.sha.slice(0, 8)}, which is not an ancestor of HEAD — history was probably rewritten. Run: git lex sync`
              : `the graph is ${n} commit${n === 1 ? '' : 's'} behind this repo. It was built at ${g.sha.slice(0, 8)}. Saving does not advance it; only \u2018git lex sync\u2019 does.`,
        }
      }
      case 'unplaceable':
        return {
          mark: '\u25cb',
          cls: 'unknown',
          title:
            'a graph store exists but nothing on disk says which commit it was built at, so its age cannot be read without asking a running server. Not the same as up to date.',
        }
      case 'never-synced':
        return {
          mark: '\u2013',
          cls: 'unknown',
          title: 'no graph store at all — this repo has never been synced.',
        }
    }
  }

  /** How many rows we can actually place, and how many of those are behind.
   *  Stated as "N of M" rather than a bare N, because the denominator is the
   *  part that says whether the number is worrying. */
  let placeable = $derived(
    repos.filter((r) => r.graph && (r.graph.state === 'current' || r.graph.state === 'behind')).length,
  )
  let behindCount = $derived(repos.filter((r) => r.graph?.state === 'behind').length)
  let behindCommits = $derived(
    repos.reduce((a, r) => a + (r.graph?.state === 'behind' ? (r.graph.commits ?? 0) : 0), 0),
  )

  /** The list, split by what kind of repo each row is.
   *
   *  Souls first because they are the common case here, then other kits, then
   *  plain markdown repos. Every group is always rendered when it has rows —
   *  a single plain repo among twenty souls is exactly the row most likely to
   *  be forgotten, and a header is what stops it disappearing into the
   *  majority. Empty groups draw nothing; a heading over no rows is noise. */
  const GROUPS: { key: string; label: string; note: string }[] = [
    { key: 'soul', label: 'souls', note: 'running the soul kit — journal, notes, pursuits' },
    { key: 'kitted', label: 'other kits', note: 'git-lex with a kit that is not the soul kit' },
    { key: 'plain', label: 'markdown', note: 'the base case: markdown in git, no kit installed' },
  ]
  let grouped = $derived(
    GROUPS.map((g) => ({
      ...g,
      rows: repos.filter((r) => (r.family?.family ?? 'plain') === g.key),
    })).filter((g) => g.rows.length > 0),
  )

  function dot(r: RepoProbe): string {
    const s = servers[r.path]
    if (!s) return ''
    if (s.state === 'ready') return 'on'
    if (s.state === 'starting') return 'starting'
    return 'bad'
  }
</script>

<aside class="rail">
  <section class="block">
    <h2>repos <span class="n">{repos.length}</span></h2>
    <ul class="repos">
      {#each grouped as g (g.key)}
        {#if grouped.length > 1}
          <li class="grouphead" title={g.note}>{g.label} <span class="gn">{g.rows.length}</span></li>
        {/if}
      {#each g.rows as r (r.path)}
        <li>
          <button
            class="repo"
            class:active={current?.path === r.path}
            disabled={busy === r.path}
            onclick={() => onpick(r)}
          >
            <span class="status {dot(r)}"></span>
            <span class="rname">{r.name}</span>
            <span class="rkit">{kitLabel(r.kit)}</span>
            {#if stale(r)}
              {@const st = stale(r)!}
              <span class="rstale {st.cls}" title={st.title}>{st.mark}</span>
            {/if}
            <span class="rwhen" title={r.recency_source === 'last-used'
              ? 'from the registry — a record of it being opened'
              : "the repo's own last commit; the registry had no last_used for it"}>
              {ago(r.recency)}{r.recency_source === 'head-commit' ? '*' : ''}
            </span>
          </button>
        </li>
      {/each}
      {/each}
    </ul>
    <p class="foot">* ordered by last commit — the registry had no record of it being opened</p>
    {#if behindCount > 0}
      <p class="foot warn" title="git lex save commits your work; it does not rebuild the graph. Only git lex sync does. Read from each repo's spine filename, which is named for the commit it was built at.">
        &#8634; {behindCount} of {placeable} graphs are behind their repo — {behindCommits} commits unindexed. Run <code>git lex sync</code> in each.
      </p>
    {/if}
  </section>

  <section class="block">
    <h2>view</h2>
    <div class="views">
      <button class:on={view === 'spiral'} onclick={() => onview('spiral')}>whole soul</button>
      <button
        class:on={view === 'neighbourhood'}
        disabled={!hasSelection}
        title={hasSelection ? '' : 'select a node first'}
        onclick={() => onview('neighbourhood')}
      >neighbourhood</button>
    </div>
  </section>

  <section class="block">
    <h2>find</h2>
    <input
      class="search"
      type="search"
      placeholder="label or id…"
      value={search}
      oninput={(e) => onsearch((e.currentTarget as HTMLInputElement).value)}
    />
    {#if search}
      <p class="foot" class:none={matchCount === 0}>
        {matchCount} {matchCount === 1 ? 'match' : 'matches'}
        {#if matchCount === 0}— nothing on the stage is hidden by this, only dimmed{/if}
      </p>
    {/if}
  </section>

  <section class="block grow">
    <h2>
      classes
      {#if meta}<span class="n">{meta.classes.length}</span>{/if}
      <button class="all" onclick={onallclasses}>all</button>
    </h2>
    {#if meta}
      <ul class="classes">
        {#each meta.classes as c, i (c.uri)}
          <li>
            <button
              class="cls"
              class:off={!visibleClasses.has(i)}
              onclick={() => ontoggle(i)}
              title={c.uri}
            >
              <span class="swatch" style="background:{c.color}"></span>
              <span class="cname">{c.name}</span>
              <span class="ccount">{c.count}</span>
            </button>
            <button class="only" onclick={() => onlyclass(i)} title="show only this class">only</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="foot">no soul open</p>
    {/if}
  </section>

  <!-- Link kinds. Separate from classes because they answer a different
       question: classes are what a document IS, predicates are how documents
       are JOINED, and on a soul where one predicate is 80% of the links the
       picture is unreadable until you can switch it off. -->
  <section class="block grow">
    <h2>
      link kinds
      {#if meta}<span class="n">{meta.predicates.length}</span>{/if}
      <button class="all" onclick={onallpredicates}>all</button>
    </h2>
    {#if meta && meta.predicates.length}
      <ul class="classes">
        {#each meta.predicates as p, i (p.uri)}
          <li>
            <button
              class="cls"
              class:off={!visiblePredicates.has(i)}
              onclick={() => ontogglepredicate(i)}
              title={p.uri}
            >
              <span class="tick">{visiblePredicates.has(i) ? '\u25a0' : '\u25a1'}</span>
              <span class="cname">{p.name}</span>
              <span class="ccount">{p.count}</span>
            </button>
            <button class="only" onclick={() => onlypredicate(i)} title="show only this kind">only</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="foot">no links drawn</p>
    {/if}
  </section>
</aside>

<style>
  .rail {
    grid-area: left;
    border-right: 1px solid var(--rule);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--paper);
  }
  .block { border-bottom: 1px solid var(--rule); padding: 0.6rem 0.7rem; min-height: 0; }
  .block.grow { flex: 1; display: flex; flex-direction: column; overflow: hidden; }

  h2 {
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--ink-faint);
    margin: 0 0 0.4rem;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .n { color: var(--ink-faint); }
  .all {
    margin-left: auto; border: none; background: none; padding: 0;
    font-size: 10px; text-transform: uppercase; letter-spacing: 0.08em;
    text-decoration: underline; color: var(--ink-faint);
  }
  .all:hover { background: none; color: var(--ink); }

  ul { list-style: none; margin: 0; padding: 0; }
  .repos { max-height: 34vh; overflow-y: auto; }
  /* Segments the list without competing with the rows. The count matters as
     much as the name: "markdown 1" says the base case is present and rare,
     which a bare heading would not. */
  .grouphead {
    font-size: 9px; letter-spacing: 0.09em; text-transform: uppercase;
    color: var(--ink-faint); padding: 0.5rem 0.3rem 0.15rem;
    border-bottom: 1px solid var(--rule); margin-bottom: 0.15rem;
  }
  .grouphead:first-child { padding-top: 0.1rem; }
  .gn { float: right; font-variant-numeric: tabular-nums; }

  .repo {
    display: grid;
    grid-template-columns: 8px 1fr auto;
    grid-template-areas: "s n k" "s w g";
    gap: 0 0.4rem;
    width: 100%;
    text-align: left;
    border: none;
    background: none;
    padding: 0.25rem 0.3rem;
    font-size: 12px;
  }
  .repo:hover:not(:disabled) { background: var(--paper-tint); color: inherit; }
  .repo.active { background: var(--ink); color: var(--paper); }
  .repo.active .rkit, .repo.active .rwhen { color: rgba(255,255,255,0.7); }

  .status { grid-area: s; width: 6px; height: 6px; border-radius: 50%; margin-top: 5px; background: transparent; border: 1px solid var(--rule); }
  .status.on { background: var(--live); border-color: var(--live); }
  .status.starting { background: var(--warn); border-color: var(--warn); }
  .status.bad { background: var(--dead); border-color: var(--dead); }

  .rname { grid-area: n; font-family: var(--display); font-size: 13px; }
  .rkit { grid-area: k; font-size: 10px; color: var(--ink-faint); }
  .rwhen { grid-area: w; font-size: 10px; color: var(--ink-faint); }

  /* The staleness marker. Muted on purpose: 13 of 15 souls carry one, so a
     loud treatment would make the normal state of the machine look like an
     outage. It reads as a annotation, and the tooltip carries the fix. */
  .rstale { grid-area: g; font-size: 10px; font-variant-numeric: tabular-nums; justify-self: end; }
  .rstale.behind { color: var(--warn); }
  .rstale.unknown { color: var(--ink-faint); }
  .repo.active .rstale { color: rgba(255,255,255,0.75); }

  .views { display: flex; gap: 0.3rem; }
  .views button { flex: 1; font-size: 11px; padding: 0.2rem 0.3rem; border: 1px solid var(--rule); }
  .views button.on { background: var(--ink); color: var(--paper); border-color: var(--ink); }

  .search {
    width: 100%; font: inherit; font-size: 12px;
    border: 1px solid var(--rule); padding: 0.25rem 0.4rem; background: var(--paper); color: var(--ink);
  }
  .search:focus { outline: 1px solid var(--ink); border-color: var(--ink); }

  .classes { flex: 1; overflow-y: auto; }
  .classes li { display: flex; align-items: center; gap: 0.2rem; }
  .cls {
    flex: 1; display: flex; align-items: center; gap: 0.4rem;
    border: none; background: none; padding: 0.15rem 0.2rem;
    font-size: 11px; text-align: left; min-width: 0;
  }
  .cls:hover { background: var(--paper-tint); color: inherit; }
  .cls.off { opacity: 0.35; }
  .cls.off .swatch { background: transparent !important; border: 1px solid var(--ink-faint); }
  .swatch { width: 9px; height: 9px; flex: none; }
  .tick { width: 9px; flex: none; font-size: 9px; color: var(--ink-soft); }
  .cname { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ccount { color: var(--ink-faint); font-variant-numeric: tabular-nums; }
  .only {
    border: none; background: none; padding: 0 0.2rem;
    font-size: 9px; text-transform: uppercase; color: var(--ink-faint); opacity: 0;
  }
  .classes li:hover .only { opacity: 1; }
  .only:hover { background: none; color: var(--ink); text-decoration: underline; }

  .foot { font-size: 10px; color: var(--ink-faint); margin: 0.35rem 0 0; }
  .foot.none { color: var(--warn); }
  .foot.warn { color: var(--warn); }
  .foot code { font-family: var(--mono); font-size: 10px; }
</style>
