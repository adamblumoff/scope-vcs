import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestRevisions } from '@/api/types'
import type { RequestDiscussion } from './request-discussion-types'
import {
  discussionsForRequestCommit,
  missingRequestCommitFileError,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
  requestRevisionPin,
} from './request-changes-model'

const revisions: RequestRevisions['revisions'] = [
  revision('revision-1', 1, ['commit-a', 'commit-b']),
  revision('revision-2', 4, ['commit-c']),
]

test('defaults to the newest revision and its newest commit', () => {
  const selection = requestChangeSelection(revisions, 'revision-2', {})
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
  assert.equal(selection.error, null)
})

test('keeps the newest revision selected when its inspection is unavailable', () => {
  const hiddenLatest = revision('revision-3', 6, [], 'Unavailable')
  const selection = requestChangeSelection(
    [...revisions, hiddenLatest],
    'revision-3',
    {},
  )
  assert.equal(selection.revision?.id, 'revision-3')
  assert.equal(selection.commit, null)
  assert.match(selection.error ?? '', /could not be inspected/)
})

test('an explicit revision remains pinned when a newer revision arrives', () => {
  const newer = revision('revision-3', 6, ['commit-d'])
  const selection = requestChangeSelection(
    [...revisions, newer],
    'revision-3',
    { revision: 'revision-2' },
  )
  assert.equal(selection.revision?.id, 'revision-2')
  assert.equal(selection.commit, 'commit-c')
})

test('initial selection becomes an explicit revision pin before refreshes', () => {
  const initial = requestChangeSelection(revisions, 'revision-2', {})
  assert.deepEqual(
    requestRevisionPin(initial.revision, initial.commit, undefined),
    { commit: 'commit-c', revision: 'revision-2' },
  )
  assert.equal(
    requestRevisionPin(initial.revision, initial.commit, 'revision-2'),
    null,
  )
})

test('keeps revision and commit selection consistent', () => {
  assert.deepEqual(
    requestChangeSelection(revisions, 'revision-2', {
      commit: 'commit-a',
      revision: 'revision-1',
    }),
    {
      commit: 'commit-a',
      revision: revisions[0],
      error: null,
    },
  )
  assert.match(
    requestChangeSelection(revisions, 'revision-2', {
      commit: 'commit-c',
      revision: 'revision-1',
    }).error ?? '',
    /not part of the request/,
  )
})

test('keeps a valid commit selectable when its file list is truncated', () => {
  const oversized = [revision('revision-large', 1, ['commit-large'])]
  oversized[0].commits[0].files = []
  oversized[0].commits[0].change_count = 10_001
  oversized[0].commits[0].files_truncated = true

  const selection = requestChangeSelection(
    oversized,
    'revision-large',
    { commit: 'commit-large', revision: 'revision-large' },
  )
  assert.equal(selection.commit, 'commit-large')
  assert.equal(selection.error, null)
  assert.match(
    missingRequestCommitFileError(oversized[0].commits[0]),
    /outside the bounded file list/,
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
    discussion('later', 8, { revision_id: 'revision-1', revision_position: 1, commit_oid: 'commit-b', path: null }),
    discussion('revision', 5, { revision_id: 'revision-1', revision_position: 1, commit_oid: null, path: null }),
    discussion('other', 6, { revision_id: 'revision-1', revision_position: 1, commit_oid: 'commit-a', path: null }),
  ]
  assert.deepEqual(
    discussionsForRequestCommit(discussions, revisions[0], 'commit-b')
      .map(({ id }) => id),
    ['revision', 'later'],
  )
})

function revision(
  id: string,
  position: number,
  commits: string[],
  inspection: RequestRevisions['revisions'][number]['inspection'] = 'Complete',
) {
  return {
    actor: { handle: 'adam', id: 'user-1' },
    commits: commits.map((oid) => ({
      author: 'Adam <adam@example.com>',
      authored_at_unix: position,
      change_count: 1,
      files: [{
        kind: 'Modified' as const,
        new_mode: '100644',
        new_oid: oid,
        old_mode: '100644',
        old_oid: 'base',
        path: `${oid}.txt`,
        visibility: 'Public' as const,
      }],
      message: `Commit ${oid}`,
      oid,
      parent_oids: ['base'],
      files_truncated: false,
    })),
    created_at_unix: position,
    id,
    inspection,
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
    read_through_position: openedPosition,
    reply_count: 0,
    request_id: 'request-1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
  }
}
