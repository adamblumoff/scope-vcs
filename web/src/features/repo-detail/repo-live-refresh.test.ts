import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import type { RepoLiveState } from '@/api/types'
import {
  parseRepoChangeEvent,
  takeSseMessages,
  usesVersionedRepoChangeEvents,
} from './repo-live-refresh'

test('parseRepoChangeEvent reads repo change SSE payloads', () => {
  assert.deepEqual(
    parseRepoChangeEvent(
      'event: repo-change\ndata: {"repo_id":"owner/repo","version":2,"reason":"visibility-changed"}',
    ),
    {
      reason: 'visibility-changed',
      repo_id: 'owner/repo',
      version: 2,
    },
  )
})

test('parseRepoChangeEvent ignores keepalive comments', () => {
  assert.equal(parseRepoChangeEvent(': keep-alive'), null)
})

test('takeSseMessages keeps partial message buffered', () => {
  assert.deepEqual(takeSseMessages('event: one\n\nevent: two'), {
    messages: ['event: one'],
    rest: 'event: two',
  })
})

test('public repo readers keep a live refresh stream without versioned events', () => {
  const live = repoLiveState('Public', 0)

  assert.equal(usesVersionedRepoChangeEvents(live), false)
})

test('members use versioned repo change events', () => {
  const live = repoLiveState('Member', 2)

  assert.equal(usesVersionedRepoChangeEvents(live), true)
})

function repoLiveState(
  actor: RepoLiveState['repo']['access']['actor'],
  changeVersion: number,
): RepoLiveState {
  return {
    clerk_token_template: 'scope-api',
    event_stream_url: 'http://localhost.test/v1/repos/owner/repo/events',
    repo: {
      change_version: changeVersion,
      default_visibility: 'Public',
      id: 'owner/repo',
      lifecycle_state: 'Published',
      name: 'repo',
      owner_handle: 'owner',
      open_request_count: 0,
      request_permissions: {
        can_submit_request: true,
        uses_credit_stake: actor === 'Public',
      },
      access: {
        actor,
        can_apply_changes: false,
        can_change_file_visibility: false,
        can_delete_repo: actor === 'Owner',
        can_manage_members: actor === 'Owner',
        can_push: false,
        can_read_private_files: actor !== 'Public',
      },
    },
  }
}
