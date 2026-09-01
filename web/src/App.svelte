<script lang="ts">
  import { api } from './lib/api'
  import type { ReposResponse, RepoProbe, ServerStatus } from './lib/types'
  import RepoPicker from './lib/RepoPicker.svelte'
  import RepoView from './lib/RepoView.svelte'

  let data = $state<ReposResponse | null>(null)
  let error = $state<string | null>(null)
  let servers = $state<Record<string, ServerStatus>>({})
  let busy = $state<string | null>(null)
  let selected = $state<RepoProbe | null>(null)

  async function load() {
    try {
      data = await api.repos()
      error = null
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
  }

  // Health is asked, never remembered. A page left open finds out its server
  // died, rather than continuing to render the last data it had and looking
  // entirely fine — which is what happened on 2026-08-27 and cost a night.
  async function pollServers() {
    try {
      const list = await api.servers()
      servers = Object.fromEntries(list.map((s) => [s.path, s]))
    } catch {
      /* the front door itself is unreachable; the next tick will say so */
    }
  }

  $effect(() => {
    load()
    pollServers()
    const t = setInterval(pollServers, 5000)
    return () => clearInterval(t)
  })

  // A soul is addressable by its genesis sha, so a link to one survives the
  // repo being moved or renamed on disk. The path would not: the path is
  // where the repo is sitting today, the genesis sha is which repo it is.
  $effect(() => {
    if (!data) return
    const want = location.hash.replace(/^#/, '')
    if (!want) {
      if (selected) selected = null
      return
    }
    if (selected?.genesis_sha === want) return
    const repo = data.repos.find((r) => r.genesis_sha === want)
    if (repo) openRepo(repo)
  })

  function show(repo: RepoProbe | null) {
    selected = repo
    const hash = repo?.genesis_sha ? `#${repo.genesis_sha}` : ''
    if (location.hash !== hash) history.replaceState(null, '', hash || location.pathname)
  }

  async function openRepo(repo: RepoProbe) {
    busy = repo.path
    try {
      const st = await api.open(repo.path)
      servers = { ...servers, [repo.path]: st }
      if (st.state === 'ready') show(repo)
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    } finally {
      busy = null
    }
  }
</script>

{#if error}
  <div class="bar error">{error} <button onclick={load}>retry</button></div>
{/if}

{#if selected}
  <RepoView
    repo={selected}
    status={servers[selected.path]}
    onback={() => show(null)}
  />
{:else if data}
  <RepoPicker {data} {servers} {busy} onopen={openRepo} />
{:else if !error}
  <div class="bar">reading the registry…</div>
{/if}

<style>
  .bar {
    font-family: var(--mono);
    padding: 1rem 1.5rem;
    color: var(--ink-soft);
  }
  .error {
    background: #fdf5f5;
    color: var(--dead);
    border-bottom: 1px solid #f0d4d4;
  }
</style>
