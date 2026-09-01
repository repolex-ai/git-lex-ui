/** How the registry's `kit` field is actually populated, measured across the
 *  16 live repos on this machine: some hold a bare name (`soul`), most hold a
 *  full GitHub slug (`repolex-ai/git-lex-kit-soul`), one holds nothing. Show
 *  the short form, keep the raw for the detail line. */
export function kitLabel(kit: string | null): string {
  if (!kit) return '—'
  const m = kit.match(/git-lex-kit-(.+)$/)
  return m ? m[1] : kit
}

const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

export function ago(iso: string | null): string {
  if (!iso) return 'never'
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const d = Date.now() - t
  if (d < MINUTE) return 'just now'
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m ago`
  if (d < DAY) return `${Math.floor(d / HOUR)}h ago`
  const days = Math.floor(d / DAY)
  if (days < 30) return `${days}d ago`
  const months = Math.floor(days / 30)
  return months < 12 ? `${months}mo ago` : `${Math.floor(days / 365)}y ago`
}

/** The sentence that says which clock produced a timestamp. Every row that
 *  shows a time shows one of these on hover — the caption has to carry its
 *  own evidence. */
export function clockNote(source: string, iso: string | null): string {
  if (!iso) return 'no timestamp from either clock'
  switch (source) {
    case 'last-used':
      return `last opened ${iso} — from the registry's own last_used`
    case 'head-commit':
      return `last commit ${iso} — the registry had no last_used for this repo, so this is the repo's own clock, not a record of anyone opening it`
    default:
      return iso
  }
}

export function shortSha(sha: string | null, n = 8): string {
  return sha ? sha.slice(0, n) : '—'
}
