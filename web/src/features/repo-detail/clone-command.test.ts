import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import { credentialedCloneCommand, publicCloneCommand } from './clone-command'

test('publicCloneCommand uses the plain Git remote URL', () => {
  assert.equal(
    publicCloneCommand({
      git_remote_url: 'https://scope.example/git/adam/scope-vcs',
    }),
    'git clone https://scope.example/git/adam/scope-vcs',
  )
})

test('credentialedCloneCommand stores clone credentials under a clone user', () => {
  assert.equal(
    credentialedCloneCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_clone_secret',
    ),
    '"protocol=https`nhost=scope.example`npath=git/adam/scope-vcs`nusername=scope-clone`npassword=scope_clone_secret`n`n" | git -c "credential.https://scope.example.useHttpPath=true" credential approve; git clone -c "credential.https://scope.example.useHttpPath=true" -c "http.proactiveAuth=basic" https://scope-clone@scope.example/git/adam/scope-vcs',
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
    '"protocol=http`nhost=localhost:8080`npath=git/local/scope-vcs`nusername=scope-clone`npassword=scope_clone_secret`n`n" | git -c "credential.http://localhost:8080.useHttpPath=true" credential approve; git clone -c "credential.http://localhost:8080.useHttpPath=true" -c "http.proactiveAuth=basic" http://scope-clone@localhost:8080/git/local/scope-vcs',
  )
})

test('credentialedCloneCommand escapes PowerShell credential values', () => {
  assert.equal(
    credentialedCloneCommand(
      {
        git_remote_url: 'https://old-user@scope.example/git/adam/scope-vcs',
      },
      'scope_clone_$"tick`',
    ),
    '"protocol=https`nhost=scope.example`npath=git/adam/scope-vcs`nusername=scope-clone`npassword=scope_clone_`$`"tick```n`n" | git -c "credential.https://scope.example.useHttpPath=true" credential approve; git clone -c "credential.https://scope.example.useHttpPath=true" -c "http.proactiveAuth=basic" https://scope-clone@scope.example/git/adam/scope-vcs',
  )
})
