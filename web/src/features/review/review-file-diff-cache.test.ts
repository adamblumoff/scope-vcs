import assert from 'node:assert/strict'
import test from 'node:test'
import type { ReviewFileDiff } from '@/api/types'
import type { FileDiffMetadata } from '@pierre/diffs'
import {
  parsedDiffCacheStats,
  parsedDiffForReviewFile,
  resetParsedDiffCache,
} from './review-file-diff-cache'

test('caches parsed text diffs and bounds them at twelve entries', () => {
  resetParsedDiffCache()
  const firstDiff = textDiff(0)
  const first = parsedDiffForReviewFile(firstDiff, 'diff-0', parseDiff)
  assert.ok(first)
  assert.strictEqual(parsedDiffForReviewFile(firstDiff, 'diff-0', parseDiff), first)

  for (let index = 1; index < 13; index += 1) {
    assert.ok(parsedDiffForReviewFile(textDiff(index), `diff-${index}`, parseDiff))
  }

  assert.equal(parsedDiffCacheStats().entries, 12)
})

test('accounts parsed entries by the serialized source diff size', () => {
  resetParsedDiffCache()
  const source = textDiff(1)

  assert.ok(parsedDiffForReviewFile(source, 'weighted', parseDiff))
  assert.equal(
    parsedDiffCacheStats().totalBytes,
    JSON.stringify(source).length * 2,
  )
})

test('does not cache binary or unkeyed diffs', () => {
  resetParsedDiffCache()
  assert.equal(parsedDiffForReviewFile(binaryDiff(), 'binary', parseDiff), null)
  assert.ok(parsedDiffForReviewFile(textDiff(1), undefined, parseDiff))
  assert.deepEqual(parsedDiffCacheStats(), { entries: 0, totalBytes: 0 })
})

function textDiff(index: number): ReviewFileDiff {
  return {
    kind: 'Modified',
    new_content: { kind: 'text', text: `after-${index}\n` },
    new_mode: '100644',
    old_content: { kind: 'text', text: 'before\n' },
    old_mode: '100644',
    path: `/${index}.txt`,
  }
}

function binaryDiff(): ReviewFileDiff {
  return {
    kind: 'Modified',
    new_content: { kind: 'binary', oid: 'new', size_bytes: 20 },
    new_mode: '100644',
    old_content: { kind: 'binary', oid: 'old', size_bytes: 10 },
    old_mode: '100644',
    path: '/asset.bin',
  }
}

function parseDiff(diff: ReviewFileDiff): FileDiffMetadata | null {
  if (
    diff.old_content?.kind === 'binary' ||
    diff.new_content?.kind === 'binary'
  ) {
    return null
  }
  return { hunks: [], name: diff.path } as unknown as FileDiffMetadata
}
