import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummary, HistoryPage } from '@/api/types'
import {
  appendHistoryPage,
  historySummary,
  type LoadedHistory,
} from './history-pagination'

test('appends three pages without dropping any of 120 updates', () => {
  const allEntries = Array.from({ length: 120 }, (_, index) => entry(index))
  let loaded: LoadedHistory = page(allEntries.slice(0, 50), 'cursor-50')
  loaded = appendHistoryPage(loaded, page(allEntries.slice(50, 100), 'cursor-100'))
  loaded = appendHistoryPage(loaded, page(allEntries.slice(100), null))

  assert.equal(loaded.entries.length, 120)
  assert.deepEqual(loaded.entries.map((item) => item.id), allEntries.map((item) => item.id))
  assert.equal(loaded.next_cursor, null)
})

test('does not duplicate an overlapping boundary entry', () => {
  const first = page([entry(0), entry(1)], 'cursor-2')
  const loaded = appendHistoryPage(first, page([entry(1), entry(2)], null))
  assert.deepEqual(loaded.entries.map((item) => item.id), ['entry-0', 'entry-1', 'entry-2'])
})

test('describes a partial page as the most recent updates', () => {
  assert.equal(historySummary([entry(0)], true), '1 most recent updates')
  assert.equal(historySummary([entry(0)], false), '1 update')
})

function page(entries: HistoryEntrySummary[], nextCursor: string | null): HistoryPage {
  return {
    audience: 'public',
    entries,
    generation: 'generation-1',
    next_cursor: nextCursor,
    repo_id: 'scope/demo',
    view_key: 'public',
  }
}

function entry(index: number): HistoryEntrySummary {
  return {
    author: null,
    change_count: 1,
    id: `entry-${index}`,
    kind: 'push',
    message: `Update ${index}`,
    parent_id: index === 0 ? null : `entry-${index - 1}`,
    source_id: `source-${index}`,
  }
}
