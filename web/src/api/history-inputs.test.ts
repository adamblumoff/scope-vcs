import assert from 'node:assert/strict'
import test from 'node:test'
import {
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from './history-inputs'

test('normalizes an optional history cursor', () => {
  assert.deepEqual(parseHistoryPageInput({
    audience: 'private',
    before: '  cursor-50 ',
    owner: ' scope ',
    repo: ' vcs ',
  }), {
    audience: 'private',
    before: 'cursor-50',
    owner: 'scope',
    repo: 'vcs',
  })
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs' }).before, null)
  assert.equal(parseHistoryPageInput({ owner: 'scope', repo: 'vcs' }).audience, null)
})

test('validates direct history entry and file diff requests', () => {
  assert.equal(parseHistoryEntryDetailInput({
    entry: ' update-100 ', owner: 'scope', repo: 'vcs',
  }).entry, 'update-100')
  assert.equal(parseHistoryEntryFileDiffInput({
    entry: 'update-100', owner: 'scope', path: ' /README.md ', repo: 'vcs',
  }).path, '/README.md')
  assert.throws(
    () => parseHistoryEntryDetailInput({ entry: ' ', owner: 'scope', repo: 'vcs' }),
    /history entry id is required/,
  )
})
