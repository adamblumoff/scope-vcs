import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { gitCredentialApproveCommand, setupCommand } from './commands'

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
    'git config "credential.https://scope.example.useHttpPath" true; "protocol=https`nhost=scope.example`npath=git/adam/scope-vcs`nusername=scope`npassword=scope_git_secret`n`n" | git credential approve; git remote remove scope 2>$null; git remote add scope https://scope@scope.example/git/adam/scope-vcs; git push scope HEAD:trunk',
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
    'git config "credential.https://scope.example.useHttpPath" true; "protocol=https`nhost=scope.example`npath=git/adam/scope-vcs`nusername=scope`npassword=scope_git_`$`"tick```n`n" | git credential approve; git remote remove scope 2>$null; git remote add scope https://scope@scope.example/git/adam/scope-vcs; git push scope HEAD:trunk',
  )
})

test('gitCredentialApproveCommand stores only the Scope credential', () => {
  assert.equal(
    gitCredentialApproveCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_git_secret',
    ),
    'git config "credential.https://scope.example.useHttpPath" true; "protocol=https`nhost=scope.example`npath=git/adam/scope-vcs`nusername=scope`npassword=scope_git_secret`n`n" | git credential approve',
  )
})

