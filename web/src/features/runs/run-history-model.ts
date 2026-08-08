import type { RepoRunHistoryPage } from '@/api/types'

export function refreshRunHistoryPage(
  current: RepoRunHistoryPage | null,
  next: RepoRunHistoryPage,
  preserveLoadedCursor: boolean,
): RepoRunHistoryPage {
  return {
    next_cursor: preserveLoadedCursor && current
      ? current.next_cursor
      : next.next_cursor,
    runs: mergeRunHistory(next.runs, current?.runs ?? []),
  }
}

export function mergeRunHistory(
  first: RepoRunHistoryPage['runs'],
  second: RepoRunHistoryPage['runs'],
) {
  const seen = new Set(first.map((run) => run.id))
  return [...first, ...second.filter((run) => !seen.has(run.id))]
}
