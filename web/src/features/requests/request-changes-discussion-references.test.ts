import assert from 'node:assert/strict'
import test from 'node:test'
import type { LoadDiscussionsInput } from './request-discussion-api'
import {
  loadCompleteDiscussionReferencePage,
  loadCompleteDiscussionReferencePages,
} from './request-changes-discussion-references'
import type { RequestDiscussion, RequestDiscussionPage } from './request-discussion-types'

test('loads discussion references through the final cursor', async () => {
  const cursors: Array<string | undefined> = []
  const page = await loadCompleteDiscussionReferencePage(
    requestInput(),
    async (input) => {
      cursors.push(input.cursor)
      return input.cursor
        ? discussionPage(['last'], null, 41)
        : discussionPage(
            Array.from({ length: 100 }, (_, index) => `first-${index}`),
            'next-page',
            41,
          )
    },
  )

  assert.deepEqual(cursors, [undefined, 'next-page'])
  assert.equal(page.discussions.length, 101)
  assert.equal(page.discussions.at(-1)?.id, 'last')
  assert.equal(page.next_cursor, null)
  assert.equal(page.snapshot_version, 41)
})

test('rejects a repeated discussion cursor instead of looping', async () => {
  await assert.rejects(
    loadCompleteDiscussionReferencePage(requestInput(), async () =>
      discussionPage([], 'repeated', 1)),
    /repeated a cursor/,
  )
})

test('rejects a changed snapshot version instead of mixing pages', async () => {
  await assert.rejects(
    loadCompleteDiscussionReferencePage(requestInput(), async (input) =>
      input.cursor
        ? discussionPage(['later'], null, 2)
        : discussionPage(['first'], 'next', 1)),
    /changed snapshot version/,
  )
})

test('loads commits sequentially and isolates a failed commit', async () => {
  const calls: string[] = []
  const errors: unknown[] = []
  const byCommit = await loadCompleteDiscussionReferencePages(
    [
      { input: { ...requestInput(), commit_oid: 'first' }, key: 'first' },
      { input: { ...requestInput(), commit_oid: 'failed' }, key: 'failed' },
      { input: { ...requestInput(), commit_oid: 'last' }, key: 'last' },
    ],
    async (input) => {
      calls.push(`${input.commit_oid}:${input.cursor ?? 'first'}`)
      if (input.commit_oid === 'failed') throw new Error('unavailable')
      return input.cursor
        ? discussionPage([`${input.commit_oid}-2`], null, 1)
        : discussionPage([`${input.commit_oid}-1`], 'next', 1)
    },
    (error) => errors.push(error),
  )

  assert.deepEqual(calls, [
    'first:first',
    'first:next',
    'failed:first',
    'last:first',
    'last:next',
  ])
  assert.deepEqual(byCommit.first?.discussions.map(({ id }) => id), ['first-1', 'first-2'])
  assert.equal(byCommit.failed, null)
  assert.deepEqual(byCommit.last?.discussions.map(({ id }) => id), ['last-1', 'last-2'])
  assert.equal(errors.length, 1)
})

function requestInput(): LoadDiscussionsInput {
  return {
    commit_oid: 'a'.repeat(40),
    limit: 100,
    owner: 'owner',
    repo: 'repo',
    request_id: 'request-1',
    revision_id: 'revision-1',
  }
}

function discussionPage(
  ids: string[],
  nextCursor: string | null,
  snapshotVersion: number,
): RequestDiscussionPage {
  return {
    discussions: ids.map(discussion),
    next_cursor: nextCursor,
    snapshot_version: snapshotVersion,
  }
}

function discussion(id: string, index: number): RequestDiscussion {
  return {
    anchor: null,
    author: { handle: 'scope', id: 'user-1' },
    body_markdown: id,
    client_discussion_id: id,
    created_at_unix: index,
    id,
    last_activity_position: index,
    latest_replies: [],
    opened_position: index,
    read_through_position: index,
    reply_count: 0,
    request_id: 'request-1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}
