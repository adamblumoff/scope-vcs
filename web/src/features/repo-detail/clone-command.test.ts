import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { credentialedCloneCommand, publicCloneCommand } from './clone-command'

const cloneSource = {
  git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
}

const posixCredentialStoreSetup =
  "mkdir -p ~/.config/scope && chmod 700 ~/.config/scope && git config --global --replace-all 'credential.https://scope.example.helper' 'store --file ~/.config/scope/git-credentials' && git config --global --replace-all 'credential.https://scope.example.useHttpPath' 'true'"

const posixLocalCredentialStoreSetup =
  "mkdir -p ~/.config/scope && chmod 700 ~/.config/scope && git config --global --replace-all 'credential.http://localhost:8080.helper' 'store --file ~/.config/scope/git-credentials' && git config --global --replace-all 'credential.http://localhost:8080.useHttpPath' 'true'"

const powerShellCredentialStoreSetup =
  "$scopeCredentialDir = ($env:USERPROFILE -replace '\\\\', '/') + '/.config/scope'; New-Item -ItemType Directory -Force $scopeCredentialDir | Out-Null; $scopeCredentialFile = \"$scopeCredentialDir/git-credentials\"; git config --global --replace-all 'credential.https://scope.example.helper' \"store --file `\"$scopeCredentialFile`\"\"; git config --global --replace-all 'credential.https://scope.example.useHttpPath' 'true'"

test('publicCloneCommand defaults to Bash/Zsh and uses the plain Git remote URL', () => {
  assert.equal(
    publicCloneCommand({
      git_remote_url: 'https://scope.example/git/adam/scope-vcs',
    }),
    "git clone 'https://scope.example/git/adam/scope-vcs'",
  )
})

test('publicCloneCommand renders PowerShell on request', () => {
  assert.equal(
    publicCloneCommand(
      {
        git_remote_url: 'https://scope.example/git/adam/scope-vcs',
      },
      'powershell',
    ),
    "git clone 'https://scope.example/git/adam/scope-vcs'",
  )
})

test('credentialedCloneCommand defaults to Bash/Zsh and stores canonical Git credentials', () => {
  assert.equal(
    credentialedCloneCommand(cloneSource, 'scope_git_secret'),
    `${posixCredentialStoreSetup} && printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope' 'password=scope_git_secret' '' | git credential approve && git clone -c 'http.proactiveAuth=basic' 'https://scope@scope.example/git/adam/scope-vcs'`,
  )
})

test('credentialedCloneCommand stores local http Git credentials', () => {
  assert.equal(
    credentialedCloneCommand(
      {
        git_remote_url: 'http://localhost:8080/git/local/scope-vcs',
      },
      'scope_git_secret',
    ),
    `${posixLocalCredentialStoreSetup} && printf '%s\\n' 'protocol=http' 'host=localhost:8080' 'path=git/local/scope-vcs' 'username=scope' 'password=scope_git_secret' '' | git credential approve && git clone -c 'http.proactiveAuth=basic' 'http://scope@localhost:8080/git/local/scope-vcs'`,
  )
})

test('credentialedCloneCommand renders PowerShell on request', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_git_$"tick`',
      'powershell',
    ),
    powerShellCredentialStoreSetup +
      "; @('protocol=https', 'host=scope.example', 'path=git/adam/scope-vcs', 'username=scope', 'password=scope_git_$\"tick`', '') | git credential approve; git clone -c 'http.proactiveAuth=basic' 'https://scope@scope.example/git/adam/scope-vcs'",
  )
})

test('credentialedCloneCommand escapes Bash/Zsh credential values', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_git_$"tick`; \'apostrophe',
    ),
    posixCredentialStoreSetup +
      " && printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope' 'password=scope_git_$\"tick`; '\\''apostrophe' '' | git credential approve && git clone -c 'http.proactiveAuth=basic' 'https://scope@scope.example/git/adam/scope-vcs'",
  )
})

test('credentialedCloneCommand escapes PowerShell credential values', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_git_$"tick`; \'apostrophe',
      'powershell',
    ),
    powerShellCredentialStoreSetup +
      "; @('protocol=https', 'host=scope.example', 'path=git/adam/scope-vcs', 'username=scope', 'password=scope_git_$\"tick`; ''apostrophe', '') | git credential approve; git clone -c 'http.proactiveAuth=basic' 'https://scope@scope.example/git/adam/scope-vcs'",
  )
})
