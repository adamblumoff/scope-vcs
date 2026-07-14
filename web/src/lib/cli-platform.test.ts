import assert from 'node:assert/strict'
import test from 'node:test'
import { detectCliPlatform } from './cli-platform'

test('detectCliPlatform selects Windows installers from browser headers', () => {
  assert.equal(detectCliPlatform('"Windows"'), 'windows')
  assert.equal(
    detectCliPlatform('Mozilla/5.0 (Windows NT 10.0; Win64; x64)'),
    'windows',
  )
})

test('detectCliPlatform defaults other and missing platforms to POSIX', () => {
  assert.equal(detectCliPlatform('"macOS"'), 'posix')
  assert.equal(detectCliPlatform('Mozilla/5.0 (X11; Linux x86_64)'), 'posix')
  assert.equal(detectCliPlatform(undefined), 'posix')
})
