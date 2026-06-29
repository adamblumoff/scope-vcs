import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import type { RepoLiveState } from '@/api/types'
import {
  canUseRepoLiveRefresh,
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

  assert.equal(canUseRepoLiveRefresh(live), true)
  assert.equal(usesVersionedRepoChangeEvents(live), false)
})

test('members use versioned repo change events', () => {
  const live = repoLiveState('Member', 2)

  assert.equal(canUseRepoLiveRefresh(live), true)
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
      pending_import_pending: false,
      push_blocked_by_staged_update: false,
      access: {
        actor,
        can_apply_changes: false,
        can_change_file_visibility: false,
        can_delete_repo: actor === 'Owner',
        can_manage_members: actor === 'Owner',
        can_push: false,
        can_read_private_files: actor !== 'Public',
        can_update_repo_settings: actor === 'Owner',
      },
      staged_update_pending: false,
    },
  }
}
