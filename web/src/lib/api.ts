import type { ReposResponse, ServerStatus, FileText, SyncState } from './types'

async function json<T>(r: Response): Promise<T> {
  if (!r.ok) throw new Error((await r.text()) || `${r.status} ${r.statusText}`)
  return r.json() as Promise<T>
}

export const api = {
  repos: () => fetch('/api/repos').then(json<ReposResponse>),

  servers: () => fetch('/api/servers').then(json<ServerStatus[]>),

  /** What every in-flight or finished sync is doing, keyed by repo path. */
  syncs: () => fetch('/api/sync').then(json<Record<string, SyncState>>),

  /** Start `git lex sync` in one repo. Returns as soon as it has started —
   *  lUX takes three and a half minutes — so the caller polls `syncs()`. */
  sync: (path: string) =>
    fetch('/api/sync', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ path }),
    }).then(json<SyncState>),

  /** The text of one document, read off disk.
   *
   *  Deliberately NOT a SPARQL query. The store holds a document's facts, not
   *  its prose — no query returns the words. Failure is returned rather than
   *  thrown, because "recorded in the graph but no longer on disk" is a true
   *  fact about history, not a bug, and the panel should say so rather than
   *  showing an error box. */
  file: async (genesis: string, path: string): Promise<FileText> => {
    const r = await fetch(`/api/file/${genesis}?path=${encodeURIComponent(path)}`)
    if (!r.ok) return { path, error: (await r.text()) || `${r.status}`, text: null, bytes: 0 }
    const d = (await r.json()) as { path: string; bytes: number; text: string }
    return { ...d, error: null }
  },

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
