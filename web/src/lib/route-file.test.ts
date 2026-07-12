import assert from 'node:assert/strict'
import test from 'node:test'
import { defaultReadmePath } from './route-file'

test('prefers a repository-root README over nested README files', () => {
  assert.equal(
    defaultReadmePath([
      { path: 'docs/README.md' },
      { path: 'README.md' },
    ]),
    'README.md',
  )
})

test('does not auto-select a README hidden in a nested folder', () => {
  assert.equal(
    defaultReadmePath([{ path: 'src/index.ts' }, { path: 'docs/readme.md' }]),
    undefined,
  )
})
