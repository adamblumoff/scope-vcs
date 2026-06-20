import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { setupView } from './setup-view'

test('setupView builds the Scope remote URL from API base and repo path', () => {
  const setup = setupView('https://scope.example/', {
    git_remote_path: '/git/adam/scope-vcs',
    push_branch: 'main',
    remote_name: 'scope',
    push_enabled: true,
    repo: {
      default_visibility: 'Private',
      id: 'adam/scope-vcs',
      lifecycle_state: 'PendingFirstPush',
      name: 'scope-vcs',
      owner_handle: 'adam',
      role: 'Owner',
      staged_update_pending: false,
    },
    push_token: null,
    token: null,
  })

  assert.equal(setup.git_remote_url, 'https://scope.example/git/adam/scope-vcs')
})
