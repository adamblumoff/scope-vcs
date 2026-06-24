import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { credentialedCloneCommand, publicCloneCommand } from './clone-command'

const cloneSource = {
  git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
}

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

test('credentialedCloneCommand defaults to Bash/Zsh and stores clone credentials', () => {
  assert.equal(
    credentialedCloneCommand(cloneSource, 'scope_clone_secret'),
    "printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope-clone' 'password=scope_clone_secret' '' | git -c 'credential.https://scope.example.useHttpPath=true' credential approve && git clone -c 'credential.https://scope.example.useHttpPath=true' -c 'http.proactiveAuth=basic' 'https://scope-clone@scope.example/git/adam/scope-vcs'",
  )
})

test('credentialedCloneCommand stores local http clone credentials', () => {
  assert.equal(
    credentialedCloneCommand(
      {
        git_remote_url: 'http://localhost:8080/git/local/scope-vcs',
      },
      'scope_clone_secret',
    ),
    "printf '%s\\n' 'protocol=http' 'host=localhost:8080' 'path=git/local/scope-vcs' 'username=scope-clone' 'password=scope_clone_secret' '' | git -c 'credential.http://localhost:8080.useHttpPath=true' credential approve && git clone -c 'credential.http://localhost:8080.useHttpPath=true' -c 'http.proactiveAuth=basic' 'http://scope-clone@localhost:8080/git/local/scope-vcs'",
  )
})

test('credentialedCloneCommand renders PowerShell on request', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_clone_$"tick`',
      'powershell',
    ),
    '@(\'protocol=https\', \'host=scope.example\', \'path=git/adam/scope-vcs\', \'username=scope-clone\', \'password=scope_clone_$"tick`\', \'\') | git -c \'credential.https://scope.example.useHttpPath=true\' credential approve; git clone -c \'credential.https://scope.example.useHttpPath=true\' -c \'http.proactiveAuth=basic\' \'https://scope-clone@scope.example/git/adam/scope-vcs\'',
  )
})

test('credentialedCloneCommand escapes Bash/Zsh credential values', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_clone_$"tick`; \'apostrophe',
    ),
    "printf '%s\\n' 'protocol=https' 'host=scope.example' 'path=git/adam/scope-vcs' 'username=scope-clone' 'password=scope_clone_$\"tick`; '\\''apostrophe' '' | git -c 'credential.https://scope.example.useHttpPath=true' credential approve && git clone -c 'credential.https://scope.example.useHttpPath=true' -c 'http.proactiveAuth=basic' 'https://scope-clone@scope.example/git/adam/scope-vcs'",
  )
})

test('credentialedCloneCommand escapes PowerShell credential values', () => {
  assert.equal(
    credentialedCloneCommand(
      cloneSource,
      'scope_clone_$"tick`; \'apostrophe',
      'powershell',
    ),
    "@('protocol=https', 'host=scope.example', 'path=git/adam/scope-vcs', 'username=scope-clone', 'password=scope_clone_$\"tick`; ''apostrophe', '') | git -c 'credential.https://scope.example.useHttpPath=true' credential approve; git clone -c 'credential.https://scope.example.useHttpPath=true' -c 'http.proactiveAuth=basic' 'https://scope-clone@scope.example/git/adam/scope-vcs'",
  )
})
