import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestRevisions } from '@/api/types'
import type { RequestDiscussion } from './request-discussion-types'
import {
  discussionsForRequestCommit,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
} from './request-changes-model'

const revisions: RequestRevisions['revisions'] = [
  revision('revision-1', 1, ['commit-a', 'commit-b']),
  revision('revision-2', 4, ['commit-c']),
]

test('defaults to the newest revision and its newest commit', () => {
  const selection = requestChangeSelection(revisions, {})
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
  assert.equal(selection.unavailable, false)
})

test('defaults to the newest revision with a visible commit', () => {
  const hiddenLatest = revision('revision-3', 6, [])
  const selection = requestChangeSelection([...revisions, hiddenLatest], {})
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
})

test('keeps revision and commit selection consistent', () => {
  assert.deepEqual(
    requestChangeSelection(revisions, {
      commit: 'commit-a',
      revision: 'revision-1',
    }),
    {
      commit: 'commit-a',
      revision: revisions[0],
      unavailable: false,
    },
  )
  assert.equal(
    requestChangeSelection(revisions, {
      commit: 'commit-c',
      revision: 'revision-1',
    }).unavailable,
    true,
  )
})

test('lists newest request commits first', () => {
  assert.deepEqual(
    orderedRequestCommits(revisions).map(({ projected_id }) => projected_id),
    [
      'revision-2:commit-c',
      'revision-1:commit-b',
      'revision-1:commit-a',
    ],
  )
})

test('keeps repeated commit OIDs distinct across revisions', () => {
  const repeated = [
    revision('revision-1', 1, ['commit-a']),
    revision('revision-2', 2, ['commit-a']),
  ]
  assert.deepEqual(
    orderedRequestCommits(repeated).map(({ projected_id }) => projected_id),
    ['revision-2:commit-a', 'revision-1:commit-a'],
  )
  assert.equal(
    requestCommitForListId(repeated, 'revision-2:commit-a')?.revision.id,
    'revision-2',
  )
})

test('orders matching commit and revision discussions chronologically', () => {
  const discussions = [
    discussion('later', 8, { revision_id: 'revision-1', commit_oid: 'commit-b', path: null }),
    discussion('revision', 5, { revision_id: 'revision-1', commit_oid: null, path: null }),
    discussion('other', 6, { revision_id: 'revision-1', commit_oid: 'commit-a', path: null }),
  ]
  assert.deepEqual(
    discussionsForRequestCommit(discussions, revisions[0], 'commit-b')
      .map(({ id }) => id),
    ['revision', 'later'],
  )
})

function revision(id: string, position: number, commits: string[]) {
  return {
    actor: { handle: 'adam', id: 'user-1' },
    commits: commits.map((oid) => ({
      author: 'Adam <adam@example.com>',
      authored_at_unix: position,
      change_count: 1,
      message: `Commit ${oid}`,
      oid,
      parent_oids: ['base'],
    })),
    commits_truncated: false,
    created_at_unix: position,
    id,
    new_head_oid: commits.at(-1) ?? 'base',
    old_head_oid: 'base',
    position,
  }
}

function discussion(
  id: string,
  openedPosition: number,
  anchor: NonNullable<RequestDiscussion['anchor']>,
): RequestDiscussion {
  return {
    anchor,
    author: { handle: 'adam', id: 'user-1' },
    body_markdown: id,
    client_discussion_id: id,
    created_at_unix: openedPosition,
    id,
    last_activity_position: openedPosition,
    latest_replies: [],
    opened_position: openedPosition,
    reply_count: 0,
    request_id: 'request-1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}
