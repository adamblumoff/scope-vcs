import type { HistoryEntrySummary, HistoryPage } from '@/api/types'

export type LoadedHistory = Pick<HistoryPage, 'entries' | 'next_cursor'>

export function appendHistoryPage(
  current: LoadedHistory,
  page: HistoryPage,
): LoadedHistory {
  const knownIds = new Set(current.entries.map((entry) => entry.source_id))
  const entries = [...current.entries]
  for (const entry of page.entries) {
    if (!knownIds.has(entry.source_id)) {
      knownIds.add(entry.source_id)
      entries.push(entry)
    }
  }
  return { entries, next_cursor: page.next_cursor }
}

export function historySummary(
  entries: HistoryEntrySummary[],
  hasOlderEntries: boolean,
) {
  if (hasOlderEntries) return `${entries.length} most recent updates`
  return `${entries.length} ${entries.length === 1 ? 'update' : 'updates'}`
}
