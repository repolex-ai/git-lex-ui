<script lang="ts">
  import type { ReposResponse, RepoProbe, ServerStatus } from './types'
  import { kitLabel, ago, clockNote } from './format'

  interface Props {
    data: ReposResponse
    servers: Record<string, ServerStatus>
    onopen: (repo: RepoProbe) => void
    busy: string | null
  }
  let { data, servers, onopen, busy }: Props = $props()

  let showDropped = $state(false)

  // The two clocks, counted. A recency column on this machine is 4 rows of
  // "someone opened this" and 12 rows of "this repo committed something",
  // and those are not the same fact.
  let fromRegistry = $derived(data.counts.shown - data.counts.recency_from_commit)

  function stateLabel(s: ServerStatus | undefined): string {
    if (!s) return ''
    switch (s.state) {
      case 'ready': return `running · port ${s.port}`
      case 'starting': return 'starting…'
      case 'identity-mismatch': return 'wrong repo answering'
      case 'unreachable': return 'not answering'
      case 'exited': return 'stopped'
      case 'failed': return 'failed to start'
    }
  }
</script>

<section class="picker">
  <header>
    <h1>git-lex</h1>
    <p class="sub">
      {data.counts.shown} repos on this machine
    </p>
  </header>

  <div class="scroll-x">
    <table>
      <thead>
        <tr>
          <th class="c-name">soul</th>
          <th class="c-kit">kit</th>
          <th class="c-num">commits</th>
          <th class="c-when">last activity</th>
          <th class="c-state"></th>
        </tr>
      </thead>
      <tbody>
        {#each data.repos as r (r.path)}
          {@const s = servers[r.path]}
          <tr
            class:running={s?.state === 'ready'}
            class:trouble={s && ['identity-mismatch', 'unreachable', 'failed'].includes(s.state)}
          >
            <td class="c-name">
              <button
                class="open"
                disabled={busy === r.path}
                onclick={() => onopen(r)}
              >{r.name}</button>
              {#if !r.name_declared}
                <span class="flag" title="This repo declares no name in .lex/repo.yml — this is its directory name, which is a guess.">dir</span>
              {/if}
              <div class="path" title={r.path}>{r.path}</div>
              {#if r.warnings.length}
                {#each r.warnings as w}
                  <div class="warn">! {w}</div>
                {/each}
              {/if}
            </td>

            <td class="c-kit" title={r.kit ?? 'no kit declared'}>{kitLabel(r.kit)}</td>

            <td class="c-num">{r.commit_count ?? '—'}</td>

            <td
              class="c-when"
              class:borrowed={r.recency_source === 'head-commit'}
              title={clockNote(r.recency_source, r.recency)}
            >
              {ago(r.recency)}
              <span class="clock">
                {r.recency_source === 'last-used' ? 'opened' : r.recency_source === 'head-commit' ? 'committed' : '—'}
              </span>
            </td>

            <td class="c-state">
              {#if s}
                <span class="state">{stateLabel(s)}</span>
                {#if s.message}<div class="warn">{s.message}</div>{/if}
              {:else if busy === r.path}
                <span class="state">starting…</span>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

  <!-- Disclose what you drop. A shorter list with no account of the
       difference is how a missing repo becomes an hour of confusion. -->
  <footer>
    <p class="clocks">
      Ordered by last activity — but that column holds two different clocks.
      <strong>{fromRegistry}</strong>
      {fromRegistry === 1 ? 'row is' : 'rows are'} a real record of the repo being opened;
      <strong>{data.counts.recency_from_commit}</strong>
      {data.counts.recency_from_commit === 1 ? 'falls' : 'fall'} back to the repo's own last commit,
      because the registry has no <code>last_used</code> for them. Hover any time to see which.
    </p>

    {#if data.counts.dropped_total > 0}
      <button class="linkish" onclick={() => (showDropped = !showDropped)}>
        {showDropped ? '−' : '+'}
        {data.counts.dropped_total} registry entries not shown
        ({data.counts.dropped_path_missing} gone,
         {data.counts.dropped_scratch} scratch{data.counts.dropped_no_lex ? `, ${data.counts.dropped_no_lex} without .lex/` : ''})
      </button>

      {#if showDropped}
        <ul class="dropped">
          {#each data.dropped as d (d.path)}
            <li><code>{d.path}</code> <span class="why">— {d.reason}</span></li>
          {/each}
        </ul>
      {/if}
    {/if}

    {#if data.prune?.rewrote_file}
      <p class="prune">
        Removed {data.prune.removed.length} dead
        {data.prune.removed.length === 1 ? 'entry' : 'entries'} from
        <code>{data.registry_path}</code>, keeping {data.prune.kept}.
        The removed entries were written to <code>{data.prune.backup_path}</code> first.
        {#if data.prune.arrived_during_prune > 0}
          {data.prune.arrived_during_prune} entries were registered by other
          agents while this was happening and were left alone.
        {/if}
      </p>
    {:else if data.registry_path}
      <p class="prune faint">
        Reading <code>{data.registry_path}</code> ({data.registry_format}) — not modifying it.
      </p>
    {/if}
  </footer>
</section>

<style>
  .picker { max-width: 62rem; margin: 0 auto; padding: 3rem 1.5rem 5rem; }

  header { border-bottom: 2px solid var(--rule-strong); padding-bottom: 0.6rem; margin-bottom: 1.5rem; }
  h1 { font-size: 2.6rem; letter-spacing: -0.02em; line-height: 1; }
  .sub { margin: 0.4rem 0 0; color: var(--ink-soft); }

  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th {
    text-align: left;
    font-weight: 400;
    font-size: 11px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--ink-faint);
    border-bottom: 1px solid var(--rule);
    padding: 0 0.6rem 0.4rem 0;
  }
  td { padding: 0.7rem 0.6rem 0.7rem 0; border-bottom: 1px solid var(--rule); vertical-align: top; }
  tr.running td { background: #f4faf6; }
  tr.trouble td { background: #fdf5f5; }

  .c-num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .c-when { white-space: nowrap; }
  .c-state { white-space: nowrap; }

  .open {
    font-family: var(--display);
    font-size: 1.15rem;
    border: none;
    border-bottom: 1px solid var(--ink);
    padding: 0 0 1px;
    background: none;
  }
  .open:hover:not(:disabled) { background: var(--ink); color: var(--paper); }

  .path { color: var(--ink-faint); font-size: 11px; margin-top: 0.2rem; word-break: break-all; }
  .flag {
    font-size: 10px; letter-spacing: 0.06em; text-transform: uppercase;
    border: 1px solid var(--ink-faint); color: var(--ink-faint);
    padding: 0 0.25rem; margin-left: 0.3rem; vertical-align: 2px;
  }
  .warn { color: var(--warn); font-size: 11px; margin-top: 0.25rem; }

  /* A borrowed clock is drawn differently from a real one. The distinction
     has to survive a glance, not just a hover. */
  .clock { display: block; font-size: 10px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--ink-faint); }
  .borrowed { color: var(--ink-soft); }
  .borrowed .clock { font-style: italic; }

  .state { font-size: 11px; letter-spacing: 0.04em; color: var(--live); }
  tr.trouble .state { color: var(--dead); }

  footer { margin-top: 2rem; border-top: 1px solid var(--rule); padding-top: 1rem; }
  .clocks { color: var(--ink-soft); font-size: 12px; max-width: 46rem; }
  .clocks strong { color: var(--ink); }

  .linkish {
    border: none; padding: 0; background: none;
    text-decoration: underline; font-size: 12px; color: var(--ink-soft);
  }
  .linkish:hover { background: none; color: var(--ink); }

  .dropped { list-style: none; padding: 0.6rem 0 0; margin: 0; font-size: 11px; color: var(--ink-faint); }
  .dropped li { padding: 0.12rem 0; word-break: break-all; }
  .why { color: var(--ink-faint); }

  .prune { font-size: 11px; color: var(--ink-soft); margin-top: 0.9rem; max-width: 46rem; }
  .faint { color: var(--ink-faint); }
  code { font-size: 11px; }
</style>
