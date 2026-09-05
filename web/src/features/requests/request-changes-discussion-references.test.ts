import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestRevisions } from '@/api/types'
import type { LoadDiscussionsInput } from './request-discussion-api'
import { appendDiscussionReferencePage, loadDiscussionReferencePage, selectedDiscussionReferenceQuery } from './request-changes-discussion-references'
import type { RequestDiscussion, RequestDiscussionPage } from './request-discussion-types'

test('endless unique cursors return just the first page and preserve its cursor', async () => {
  let calls = 0
  const page = await loadDiscussionReferencePage(requestInput(), async (input, options) => {
    calls++
    assert.equal(input.limit, 100)
    assert.equal(options.maxResponseBytes, 512 * 1024)
    return discussionPage(['first'], `cursor-${calls}`, 41)
  })
  assert.equal(calls, 1)
  assert.equal(page.next_cursor, 'cursor-1')
  assert.equal(page.snapshot_version, 41)
})

test('slow pages abort transport at the whole-load deadline', async () => {
  let signal: AbortSignal | undefined
  const start = Date.now()
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async (_, options) => {
    signal = options.signal
    return new Promise(() => {})
  }), /timed out/)
  assert.equal(signal?.aborted, true)
  assert.ok(Date.now() - start < 3_000)
})

test('oversized item and byte responses are rejected without another request', async () => {
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async () =>
    discussionPage(Array.from({ length: 101 }, (_, i) => String(i)), 'more', 1)), /page limit/)
  await assert.rejects(loadDiscussionReferencePage(requestInput(), async () =>
    discussionPage(['x'.repeat(512 * 1024)], 'more', 1)), /page limit/)
})

test('continuation preserves snapshot consistency and rejects a repeated cursor', () => {
  const previous = discussionPage(['first'], 'next', 1)
  assert.throws(() => appendDiscussionReferencePage(previous, discussionPage(['later'], null, 2)), /Discussions changed/)
  assert.throws(() => appendDiscussionReferencePage(previous, discussionPage([], 'next', 1)), /repeated a cursor/)
  assert.deepEqual(appendDiscussionReferencePage(previous, discussionPage(['last'], null, 1)).discussions.map(d => d.id), ['first', 'last'])
})

test('many revisions and commits produce only the selected reference query', () => {
  const revisions = {
    review_revision_id: 'revision-19',
    revisions: Array.from({ length: 20 }, (_, i) => ({
      id: `revision-${i}`, position: i, inspection: 'Complete',
      commits: Array.from({ length: 100 }, (_, j) => ({ oid: `commit-${i}-${j}` })),
    })),
  } as RequestRevisions
  const selected = selectedDiscussionReferenceQuery({ ...requestInput(), revision_id: 'revision-3', commit_oid: 'commit-3-7' }, revisions)
  assert.equal(selected?.input.revision_id, 'revision-3')
  assert.equal(selected?.input.commit_oid, 'commit-3-7')
  assert.equal(selected?.input.include_revision_anchor, false)
  const latest = selectedDiscussionReferenceQuery({ owner: 'owner', repo: 'repo', request_id: 'request' }, revisions)
  assert.equal(latest?.input.commit_oid, 'commit-19-99')
  assert.equal(latest?.input.include_revision_anchor, true)
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
