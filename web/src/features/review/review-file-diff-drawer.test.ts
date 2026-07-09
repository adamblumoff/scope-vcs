import assert from 'node:assert/strict'
import { it } from 'node:test'
import type { ReviewFileDiff } from '@/api/types'
import { reviewContentSides } from './review-file-content'

it('keeps the text side of a mixed binary and text replacement', () => {
  const diff: ReviewFileDiff = {
    kind: 'Modified',
    new_content: { kind: 'text', text: 'readable replacement\n' },
    new_mode: '100644',
    old_content: { kind: 'binary', oid: 'abc123', size_bytes: 42 },
    old_mode: '100644',
    path: 'fixture.dat',
  }

  assert.deepEqual(reviewContentSides(diff), {
    binary: [{ label: 'Old', oid: 'abc123', sizeBytes: 42 }],
    text: [{ label: 'New', text: 'readable replacement\n' }],
  })
})
