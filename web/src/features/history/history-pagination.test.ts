import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummary, HistoryPage } from '@/api/types'
import {
  appendHistoryPage,
  historySummary,
  type LoadedHistory,
} from './history-pagination'

test('appends all 120 updates including repeated source fragments across page boundaries', () => {
  const allEntries = Array.from({ length: 120 }, (_, index) => entry(index))
  allEntries[50] = { ...allEntries[0], message: 'Later fragment of the same source' }
  allEntries[100] = { ...allEntries[50], message: 'Another fragment of the same source' }
  let loaded: LoadedHistory = page(allEntries.slice(0, 50), 'cursor-50')
  loaded = appendHistoryPage(loaded, page(allEntries.slice(50, 100), 'cursor-100'), 'cursor-50')
  loaded = appendHistoryPage(loaded, page(allEntries.slice(100), null), 'cursor-100')

  assert.equal(loaded.entries.length, 120)
  assert.deepEqual(loaded.entries, allEntries)
  assert.equal(loaded.next_cursor, null)
})

test('ignores a repeated response after its cursor has already advanced', () => {
  const first = page([entry(0), entry(1)], 'cursor-2')
  const next = page([entry(2)], null)
  const loaded = appendHistoryPage(first, next, 'cursor-2')
  assert.equal(appendHistoryPage(loaded, next, 'cursor-2'), loaded)
  assert.deepEqual(loaded.entries.map((item) => item.id), ['entry-0', 'entry-1', 'entry-2'])
})

test('ignores an older-generation response after history is reset', () => {
  const reset = page([entry(10)], 'generation-2-cursor-1')
  assert.equal(
    appendHistoryPage(reset, page([entry(2)], null), 'generation-1-cursor-2'),
    reset,
  )
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
    file_change_count: 1,
    id: `entry-${index}`,
    kind: 'push',
    message: `Update ${index}`,
    parent_id: index === 0 ? null : `entry-${index - 1}`,
    source_id: `source-${index}`,
    visibility_summary: { made_private_count: 0, made_public_count: 0 },
  }
}
