import type { RepoRunHistoryPage } from '@/api/types'
import assert from 'node:assert/strict'
import test from 'node:test'
import { refreshRunHistoryPages } from './run-history-model'

test('polling preserves an exhausted cursor after older runs are loaded', async () => {
  const current = page(null, ['new', 'old'])
  const refreshed = page('first-page-cursor', ['new'])
  const older = page(null, ['old'])

  const result = await refreshRunHistoryPages(
    current,
    async (after) => after ? older : refreshed,
  )

  assert.equal(result?.next_cursor, null)
  assert.deepEqual(result?.runs.map((run) => run.id), ['new', 'old'])
})

test('polling refetches through the loaded tail when new pages arrive', async () => {
  const current = page('old-tail-cursor', ['old-1', 'old-2', 'old-tail'])
  const pages = new Map([
    [undefined, page('new-page-2', ['new-1', 'new-2'])],
    ['new-page-2', page('new-page-3', ['new-3', 'old-1'])],
    ['new-page-3', page('refreshed-tail-cursor', ['old-2', 'old-tail'])],
  ])
  const requested: Array<string | undefined> = []

  const refreshed = await refreshRunHistoryPages(current, async (after) => {
    requested.push(after)
    return pages.get(after) ?? null
  })

  assert.deepEqual(requested, [undefined, 'new-page-2', 'new-page-3'])
  assert.deepEqual(
    refreshed?.runs.map((run) => run.id),
    ['new-1', 'new-2', 'new-3', 'old-1', 'old-2', 'old-tail'],
  )
  assert.equal(refreshed?.next_cursor, 'refreshed-tail-cursor')
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
