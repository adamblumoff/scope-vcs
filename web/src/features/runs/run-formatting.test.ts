import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { formatRunRunnerSelection } from './run-formatting'

describe('run formatting', () => {
  it('states named and mixed runner selection honestly', () => {
    assert.equal(
      formatRunRunnerSelection({ kind: 'named', name: 'linux-one' }),
      'linux-one',
    )
    assert.equal(formatRunRunnerSelection({ kind: 'mixed' }), 'multiple runners')
    assert.equal(formatRunRunnerSelection({ kind: 'any' }), 'any runner')
  })
})
