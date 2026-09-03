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

/** Run a SPARQL query against one repo's data endpoint, addressed by the
 *  repo's permanent identity rather than by a port. If that repo's endpoint is
 *  not running and verified, this fails loudly — it never falls through to
 *  whatever else happens to be listening.
 *
 *  The endpoint speaks the W3C SPARQL protocol, so results arrive as a term
 *  object per binding: `{"n":{"type":"literal","value":"W3BL0RD"}}`. Flattened
 *  here to plain strings, since every caller wants the value. An unbound
 *  variable is ABSENT from its binding rather than null, which is why callers
 *  must treat optional columns as possibly-undefined. */
export async function sparql<T = Record<string, string>>(
  genesis: string,
  query: string,
): Promise<T[]> {
  const r = await fetch(`/r/${genesis}/sparql`, {
    method: 'POST',
    headers: {
      'content-type': 'application/sparql-query',
      accept: 'application/sparql-results+json',
    },
    body: query,
  })
  if (!r.ok) throw new Error((await r.text()) || `query failed: ${r.status}`)
  const body = await r.json()
  const bindings: Record<string, { value: string }>[] = body?.results?.bindings ?? []
  return bindings.map((b) => {
    const row: Record<string, string> = {}
    for (const [k, term] of Object.entries(b)) {
      if (term && typeof term.value === 'string') row[k] = term.value
    }
    return row as T
  })
}
