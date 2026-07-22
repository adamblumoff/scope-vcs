import assert from 'node:assert/strict'
import test from 'node:test'
import {
  readRepositorySourceScroll,
  resetRepositorySourceScrollCache,
  writeRepositorySourceScroll,
} from './repository-source-scroll-cache'

test('bounds source scroll positions at sixty-four entries', () => {
  resetRepositorySourceScrollCache()
  for (let index = 0; index < 65; index += 1) {
    writeRepositorySourceScroll(`source-${index}`, index + 1)
  }

  assert.equal(readRepositorySourceScroll('source-0'), 0)
  assert.equal(readRepositorySourceScroll('source-64'), 65)
})

test('restoring scroll does not promote an entry', () => {
  resetRepositorySourceScrollCache()
  for (let index = 0; index < 64; index += 1) {
    writeRepositorySourceScroll(`source-${index}`, index + 1)
  }

  assert.equal(readRepositorySourceScroll('source-0'), 1)
  writeRepositorySourceScroll('source-64', 65)

  assert.equal(readRepositorySourceScroll('source-0'), 0)
  assert.equal(readRepositorySourceScroll('source-1'), 2)
})

test('saving scroll promotes an existing entry', () => {
  resetRepositorySourceScrollCache()
  for (let index = 0; index < 64; index += 1) {
    writeRepositorySourceScroll(`source-${index}`, index + 1)
  }

  writeRepositorySourceScroll('source-0', 101)
  writeRepositorySourceScroll('source-64', 65)

  assert.equal(readRepositorySourceScroll('source-0'), 101)
  assert.equal(readRepositorySourceScroll('source-1'), 0)
})

test('null identities do not create cache entries', () => {
  resetRepositorySourceScrollCache()
  writeRepositorySourceScroll(null, 42)

  assert.equal(readRepositorySourceScroll(null), 0)
})
