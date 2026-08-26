import assert from 'node:assert/strict'
import test from 'node:test'
import { anchorPathLabel } from './request-discussion-anchor-label'

test('a short path is left alone', () => {
  assert.equal(anchorPathLabel('src/retry.ts'), 'src/retry.ts')
})

test('a leading slash is dropped', () => {
  assert.equal(anchorPathLabel('/src/retry.ts'), 'src/retry.ts')
})

test('leading directories go first so the filename survives', () => {
  assert.equal(
    anchorPathLabel('crates/scope-domain/src/requests/discussions.rs'),
    '…/src/requests/discussions.rs',
  )
})

test('a filename longer than the budget is still returned whole', () => {
  assert.equal(
    anchorPathLabel('a/b/an-extremely-long-file-name-that-exceeds-the-budget.ts'),
    '…/an-extremely-long-file-name-that-exceeds-the-budget.ts',
  )
})

test('a bare filename never gains an ellipsis', () => {
  assert.equal(anchorPathLabel('retry.ts'), 'retry.ts')
})
