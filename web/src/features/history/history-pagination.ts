import type { HistoryEntrySummary, HistoryPage } from '@/api/types'

export type LoadedHistory = Pick<HistoryPage, 'entries' | 'next_cursor'>

export function appendHistoryPage(
  current: LoadedHistory,
  page: HistoryPage,
  before: string,
): LoadedHistory {
  if (current.next_cursor !== before) return current
  // Generation-bound cursor pages do not overlap. One source can legitimately
  // contribute multiple visibility fragments, each of which must remain visible.
  return { entries: [...current.entries, ...page.entries], next_cursor: page.next_cursor }
}

export function historySummary(
  entries: HistoryEntrySummary[],
  hasOlderEntries: boolean,
) {
  if (hasOlderEntries) return `${entries.length} most recent updates`
  return `${entries.length} ${entries.length === 1 ? 'update' : 'updates'}`
}
