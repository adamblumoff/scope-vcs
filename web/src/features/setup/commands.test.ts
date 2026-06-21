import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { setupCommand } from './commands'

test('setupCommand stores the Scope credential, resets the remote, and pushes', () => {
  assert.equal(
    setupCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
        push_branch: 'trunk',
        remote_name: 'scope',
      },
      'scope_git_secret',
    ),
    '"protocol=https`nhost=scope.example`nusername=scope`npassword=scope_git_secret`n`n" | git credential approve; git remote remove scope 2>$null; git remote add scope https://scope@scope.example/git/adam/scope-vcs; git push scope HEAD:trunk',
  )
})

test('setupCommand escapes PowerShell credential values', () => {
  assert.equal(
    setupCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
        push_branch: 'trunk',
        remote_name: 'scope',
      },
      'scope_git_$"tick`',
    ),
    '"protocol=https`nhost=scope.example`nusername=scope`npassword=scope_git_`$`"tick```n`n" | git credential approve; git remote remove scope 2>$null; git remote add scope https://scope@scope.example/git/adam/scope-vcs; git push scope HEAD:trunk',
  )
})

