import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  permissionedCloneCommand,
  publicCloneCommand,
} from './clone-command'

test('publicCloneCommand uses the plain Git remote URL', () => {
  assert.equal(
    publicCloneCommand('https://scope.example/git/public/adam/scope-vcs'),
    'git clone https://scope.example/git/public/adam/scope-vcs',
  )
})

test('permissionedCloneCommand uses the Scope CLI', () => {
  assert.equal(
    permissionedCloneCommand('adam', 'scope-vcs'),
    'scope clone adam/scope-vcs',
  )
})
