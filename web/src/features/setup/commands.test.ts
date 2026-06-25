import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { gitCredentialApproveCommand, setupCommand } from './commands'

const setupSource = {
  git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
  push_branch: 'trunk',
  remote_name: 'scope',
}

const posixCredentialStoreSetup =
  "mkdir -p ~/.config/scope && chmod 700 ~/.config/scope && git config --global --replace-all 'credential.https://scope.example.helper' 'store --file ~/.config/scope/git-credentials' && git config --global --replace-all 'credential.https://scope.example.useHttpPath' 'true'"

const powerShellCredentialStoreSetup =
  "$scopeCredentialDir = ($env:USERPROFILE -replace '\\\\', '/') + '/.config/scope'; New-Item -ItemType Directory -Force $scopeCredentialDir | Out-Null; $scopeCredentialFile = \"$scopeCredentialDir/git-credentials\"; git config --global --replace-all 'credential.https://scope.example.helper' \"store --file `\"$scopeCredentialFile`\"\"; git config --global --replace-all 'credential.https://scope.example.useHttpPath' 'true'"

test('setupCommand defaults to Bash/Zsh and pushes with Scope remote config', () => {
  assert.equal(
    setupCommand(setupSource, 'scope_git_secret'),
    `${posixCredentialStoreSetup} && printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope' 'password=scope_git_secret' '' | git credential approve && (git config --remove-section 'remote.scope' >/dev/null 2>&1 || true) && git config --replace-all 'remote.scope.url' 'https://scope@scope.example/git/adam/scope-vcs' && git config --replace-all 'remote.scope.pushurl' 'https://scope@scope.example/git/adam/scope-vcs' && git config --replace-all 'remote.scope.fetch' '+refs/heads/*:refs/remotes/scope/*' && git push 'scope' 'HEAD:trunk'`,
  )
})

test('setupCommand renders PowerShell on request', () => {
  assert.equal(
    setupCommand(
      setupSource,
      'scope_git_$"tick`',
      'powershell',
    ),
    powerShellCredentialStoreSetup +
      "; @('protocol=https', 'host=scope.example', 'path=git/adam/scope-vcs', 'username=scope', 'password=scope_git_$\"tick`', '') | git credential approve; git config --remove-section 'remote.scope' 2>$null; git config --replace-all 'remote.scope.url' 'https://scope@scope.example/git/adam/scope-vcs'; git config --replace-all 'remote.scope.pushurl' 'https://scope@scope.example/git/adam/scope-vcs'; git config --replace-all 'remote.scope.fetch' '+refs/heads/*:refs/remotes/scope/*'; git push 'scope' 'HEAD:trunk'",
  )
})

test('gitCredentialApproveCommand stores only the Scope credential in Bash/Zsh', () => {
  assert.equal(
    gitCredentialApproveCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_git_secret',
    ),
    `${posixCredentialStoreSetup} && printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope' 'password=scope_git_secret' '' | git credential approve`,
  )
})

test('gitCredentialApproveCommand escapes Bash/Zsh credential values', () => {
  assert.equal(
    gitCredentialApproveCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_git_$"tick`; \'apostrophe',
    ),
    posixCredentialStoreSetup +
      " && printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope' 'password=scope_git_$\"tick`; '\\''apostrophe' '' | git credential approve",
  )
})

test('gitCredentialApproveCommand escapes PowerShell credential values', () => {
  assert.equal(
    gitCredentialApproveCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_git_$"tick`; \'apostrophe',
      'powershell',
    ),
    powerShellCredentialStoreSetup +
      "; @('protocol=https', 'host=scope.example', 'path=git/adam/scope-vcs', 'username=scope', 'password=scope_git_$\"tick`; ''apostrophe', '') | git credential approve",
  )
})
