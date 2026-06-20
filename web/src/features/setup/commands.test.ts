import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  dualRemotePushCommands,
  gitCredentialHost,
  setupCommands,
} from './commands'

test('setupCommands replaces any URL username with the Scope credential user', () => {
  assert.deepEqual(
    setupCommands({
      git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      push_branch: 'trunk',
      remote_name: 'scope',
    }),
    [
      'git remote add scope https://scope@scope.example/git/adam/scope-vcs',
      'git push -u scope HEAD:trunk',
    ],
  )
})

test('dualRemotePushCommands keeps GitHub fetches and adds GitHub plus Scope push URLs', () => {
  assert.deepEqual(
    dualRemotePushCommands({
      git_remote_url: 'https://scope.example/git/adam/scope-vcs',
      push_branch: 'main',
      remote_name: 'scope',
    }),
    [
      'git remote get-url origin',
      'git remote set-url --add --push origin <github-remote-url>',
      'git remote set-url --add --push origin https://scope@scope.example/git/adam/scope-vcs',
      'git push origin HEAD:main',
    ],
  )
})

test('gitCredentialHost reports the credential host shown in setup help', () => {
  assert.equal(
    gitCredentialHost('https://scope@scope.example/git/adam/scope-vcs'),
    'scope.example',
  )
  assert.equal(gitCredentialHost('not a url'), 'not a url')
})

