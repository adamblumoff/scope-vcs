import type { RepoRunHistoryPage } from '@/api/types'

export async function refreshRunHistoryPages(
  pageCount: number,
  loadPage: (after?: string) => Promise<RepoRunHistoryPage | null>,
) {
  let refreshed = await loadPage()
  if (!refreshed) return null

  const requestedCursors = new Set<string>()
  for (let page = 1; page < pageCount && refreshed.next_cursor; page += 1) {
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
