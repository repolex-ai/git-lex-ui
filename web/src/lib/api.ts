import type { ReposResponse, ServerStatus } from './types'

async function json<T>(r: Response): Promise<T> {
  if (!r.ok) throw new Error((await r.text()) || `${r.status} ${r.statusText}`)
  return r.json() as Promise<T>
}

export const api = {
  repos: () => fetch('/api/repos').then(json<ReposResponse>),

  servers: () => fetch('/api/servers').then(json<ServerStatus[]>),

  open: (path: string) =>
    fetch('/api/servers/open', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path }),
    }).then(json<ServerStatus>),

  stop: (path: string) =>
    fetch('/api/servers/stop', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path }),
    }).then(json<{ stopped: boolean }>),
}

/** Run a SPARQL query against one repo's server, addressed by the repo's
 *  permanent identity rather than by a port. If that repo's server is not
 *  running and verified, this fails loudly — it never falls through to
 *  whatever else happens to be listening. */
export async function sparql<T = Record<string, string>>(
  genesis: string,
  query: string,
): Promise<T[]> {
  const r = await fetch(`/r/${genesis}/api/query`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ query }),
  })
  if (!r.ok) throw new Error((await r.text()) || `query failed: ${r.status}`)
  const body = await r.json()
  return (body.results ?? []) as T[]
}
