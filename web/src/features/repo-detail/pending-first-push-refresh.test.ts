import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import type { RepoDetail, RepoSummary } from '@/api/types'
import { shouldPollPendingFirstPush } from './pending-first-push-refresh'

test('shouldPollPendingFirstPush is scoped to the pending first-push repo page', () => {
  assert.equal(
    shouldPollPendingFirstPush(repoDetail('PendingFirstPush'), 0),
    true,
  )
  assert.equal(
    shouldPollPendingFirstPush(repoDetail('PendingFirstPush'), 1),
    false,
  )
  assert.equal(shouldPollPendingFirstPush(repoDetail('PendingPublish'), 0), false)
  assert.equal(shouldPollPendingFirstPush(repoDetail('Published'), 0), false)
  assert.equal(shouldPollPendingFirstPush(null, 0), false)
})

function repoDetail(lifecycleState: RepoSummary['lifecycle_state']): RepoDetail {
  return {
    capabilities: {
      read: true,
      write: true,
    },
    clone_remote_url: 'https://scope.example/git/owner/repo',
    files: [],
    kind: 'repo',
    projection_previews: {
      owner: null,
      public: null,
      source: 'live',
    },
    repo: {
      default_visibility: 'Private',
      id: 'owner/repo',
      lifecycle_state: lifecycleState,
      name: 'repo',
      owner_handle: 'owner',
      push_blocked_by_staged_update: false,
      role: 'Owner',
      staged_update_pending: false,
    },
    review: null,
  }
}
