import assert from 'node:assert/strict'
import test from 'node:test'
import { defaultReadmePath } from './route-file'

test('selects an exact repository-root README.html', () => {
  assert.equal(
    defaultReadmePath([
      { path: 'docs/README.html' },
      { path: 'README.html' },
    ]),
    'README.html',
  )
  assert.equal(defaultReadmePath([{ path: '/README.html' }]), 'README.html')
})

test('does not guess other README names or locations', () => {
  for (const path of [
    'README.md',
    'README',
    'README.htm',
    'readme.html',
    'docs/README.html',
    '//README.html',
  ]) {
    assert.equal(defaultReadmePath([{ path }]), undefined)
  }
})
