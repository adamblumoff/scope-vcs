import assert from 'node:assert/strict'
import test from 'node:test'
import { historyRowLabels } from './history-row-labels'

test('distinguishes commits with duplicate messages and file counts', () => {
  const firstFullId = `rv_push_${'2f91a73cf8bd'.padEnd(40, '1')}`
  const secondFullId = `rv_push_${'c831e7024a16'.padEnd(40, '2')}`
  const first = historyRowLabels(commit(
    firstFullId,
    'Fix request loading',
  ))
  const second = historyRowLabels(commit(
    secondFullId,
    'Fix request loading',
  ))

  assert.equal(first.title, second.title)
  assert.equal(first.compactId, '2f91a73cf8bd')
  assert.equal(second.compactId, 'c831e7024a16')
  assert.notEqual(first.compactId, second.compactId)
  assert.notEqual(first.ariaLabel, second.ariaLabel)
  assert.match(first.ariaLabel, new RegExp(firstFullId))
  assert.match(second.ariaLabel, new RegExp(secondFullId))
})

test('preserves message fallbacks and non-reviewed-push identities', () => {
  const blank = historyRowLabels(commit('dev-public-1', ' \nignored'))
  const multiline = historyRowLabels(commit('revision1234', ' First line \nSecond line'))
  const nonstandard = historyRowLabels(commit('rv_push_not-an-oid', 'Update'))

  assert.deepEqual(blank, {
    ariaLabel: '(no message), commit dev-public-1, 3 files',
    compactId: 'dev-public-1',
    title: '(no message)',
  })
  assert.equal(multiline.title, 'First line')
  assert.equal(multiline.compactId, 'revision1234')
  assert.equal(nonstandard.compactId, 'rv_push_not-an-oid')
})

test('uses singular file wording', () => {
  assert.equal(
    historyRowLabels(commit('dev-public-1', 'Update', 1)).ariaLabel,
    'Update, commit dev-public-1, 1 file',
  )
})

function commit(logicalCommitId: string, message: string, changeCount = 3) {
  return {
    change_count: changeCount,
    logical_commit_id: logicalCommitId,
    message,
  }
}
