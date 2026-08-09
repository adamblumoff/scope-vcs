import type { RepoRunHistoryPage } from '@/api/types'

export async function refreshRunHistoryPages(
  current: RepoRunHistoryPage,
  loadPage: (after?: string) => Promise<RepoRunHistoryPage | null>,
) {
  let refreshed = await loadPage()
  const tailId = current.runs[current.runs.length - 1]?.id
  if (!refreshed || !tailId) return refreshed

  const requestedCursors = new Set<string>()
  while (
    refreshed.next_cursor &&
    !refreshed.runs.some((run) => run.id === tailId)
  ) {
    const after = refreshed.next_cursor
    if (requestedCursors.has(after)) {
      throw new Error('run history returned a repeated cursor')
    }
    requestedCursors.add(after)
    const next = await loadPage(after)
    if (!next) return null
    refreshed = {
      next_cursor: next.next_cursor,
      runs: mergeRunHistory(refreshed.runs, next.runs),
    }
  }
  return refreshed
}

export function mergeRunHistory(
  first: RepoRunHistoryPage['runs'],
  second: RepoRunHistoryPage['runs'],
) {
  const seen = new Set(first.map((run) => run.id))
  return [...first, ...second.filter((run) => !seen.has(run.id))]
}
