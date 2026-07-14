import assert from 'node:assert/strict'
import test from 'node:test'
import type { CommitDetail, ReviewFileDiff } from '@/api/types'
import {
  historyCommitCacheKey,
  historyDiffCacheKey,
  historyResourceCacheStats,
  readHistoryCommitCache,
  readHistoryDiffCache,
  readHistoryDiffScroll,
  resetHistoryResourceCache,
  writeHistoryCommitCache,
  writeHistoryDiffCache,
  writeHistoryDiffScroll,
} from './history-resource-cache'

function commit(projectedId: string): CommitDetail {
  return {
    audience: 'public',
    author: null,
    change_count: 0,
    files: [],
    logical_commit_id: projectedId,
    message: projectedId,
    parent_projected_id: null,
    projected_id: projectedId,
    repo_id: 'scope/demo',
    view_key: 'public',
  }
}

function diff(path: string, text = 'content'): ReviewFileDiff {
  return {
    kind: 'Modified',
    new_content: { kind: 'text', text },
    new_mode: '100644',
    old_content: { kind: 'text', text: '' },
    old_mode: '100644',
    path,
  }
}

test('keys resources by immutable audience-aware identities', () => {
  const commitBase = { audience: 'public' as const, commit: 'c1', repoId: 'scope/demo' }
  assert.notEqual(
    historyCommitCacheKey(commitBase),
    historyCommitCacheKey({ ...commitBase, audience: 'private' }),
  )

  const diffBase = {
    ...commitBase,
    newOid: 'new',
    oldOid: 'old',
    path: '/README.md',
  }
  assert.notEqual(
    historyDiffCacheKey(diffBase),
    historyDiffCacheKey({ ...diffBase, newOid: 'newer' }),
  )
  assert.notEqual(
    historyDiffCacheKey(diffBase),
    historyDiffCacheKey({ ...diffBase, path: '/other.md' }),
  )
})

test('bounds commit and diff entries with least-recently-used eviction', () => {
  resetHistoryResourceCache()
  for (let index = 0; index < 60; index += 1) {
    writeHistoryCommitCache(`commit-${index}`, commit(`commit-${index}`))
  }
  for (let index = 0; index < 30; index += 1) {
    writeHistoryDiffCache(`diff-${index}`, diff(`/${index}.txt`))
  }

  assert.equal(historyResourceCacheStats().commits, 48)
  assert.equal(historyResourceCacheStats().diffs, 20)
  assert.equal(readHistoryCommitCache('commit-0'), null)
  assert.equal(readHistoryDiffCache('diff-0'), null)
  assert.equal(readHistoryDiffCache('diff-29')?.path, '/29.txt')
})

test('keeps diff scroll state with its bounded cache entry', () => {
  resetHistoryResourceCache()
  writeHistoryDiffCache('readme', diff('/README.md'))
  writeHistoryDiffScroll('readme', 420)
  assert.equal(readHistoryDiffScroll('readme'), 420)

  for (let index = 0; index < 20; index += 1) {
    writeHistoryDiffCache(`diff-${index}`, diff(`/${index}.txt`))
  }
  assert.equal(readHistoryDiffScroll('readme'), 0)
})

test('evicts large text diffs at the byte budget', () => {
  resetHistoryResourceCache()
  const largeText = 'x'.repeat(3 * 1024 * 1024)
  for (let index = 0; index < 6; index += 1) {
    writeHistoryDiffCache(`large-${index}`, diff(`/${index}.txt`, largeText))
  }

  const stats = historyResourceCacheStats()
  assert.ok(stats.diffs < 6)
  assert.ok(stats.diffBytes <= 32 * 1024 * 1024)
})
