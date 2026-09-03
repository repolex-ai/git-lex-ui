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
    <h2>souls <span class="n">{repos.length}</span></h2>
    <ul class="repos">
      {#each repos as r (r.path)}
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
            <span class="rwhen" title={r.recency_source === 'last-used'
              ? 'from the registry — a record of it being opened'
              : "the repo's own last commit; the registry had no last_used for it"}>
              {ago(r.recency)}{r.recency_source === 'head-commit' ? '*' : ''}
            </span>
          </button>
        </li>
      {/each}
    </ul>
    <p class="foot">* ordered by last commit — the registry had no record of it being opened</p>
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
  .repos { max-height: 30vh; overflow-y: auto; }

  .repo {
    display: grid;
    grid-template-columns: 8px 1fr auto;
    grid-template-areas: "s n k" "s w w";
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
</style>
