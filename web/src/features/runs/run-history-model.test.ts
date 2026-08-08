import type { RepoRunHistoryPage } from '@/api/types'
import assert from 'node:assert/strict'
import test from 'node:test'
import { refreshRunHistoryPage } from './run-history-model'

test('polling preserves an exhausted cursor after older runs are loaded', () => {
  const current = page(null, ['new', 'old'])
  const refreshed = page('first-page-cursor', ['new'])

  assert.deepEqual(refreshRunHistoryPage(current, refreshed, true), current)
})

function page(
  nextCursor: string | null,
  ids: string[],
): RepoRunHistoryPage {
  return {
    next_cursor: nextCursor,
    runs: ids.map((id, index) => ({
      can_cancel: true,
      can_retry: false,
      cancellation_requested: false,
      completed_at_unix: null,
      created_at_unix: index,
      git_oid: 'a'.repeat(40),
      id,
      runner_selection: { kind: 'any' },
      state: 'queued',
      updated_at_unix: index,
      workflow_name: 'Checks',
      workflow_path: '/.scope/runs/checks.yml',
    })),
  }
}
