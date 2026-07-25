import assert from 'node:assert/strict'
import test from 'node:test'
import { parseLoadRequestQueueInput } from './request-queue-input'

test('parseLoadRequestQueueInput normalizes pagination and search', () => {
  assert.deepEqual(
    parseLoadRequestQueueInput({
      cursor: '  ready:page-2  ',
      owner: ' scope ',
      repo: ' vcs ',
      search: '  atomic refs  ',
      section: 'ready',
    }),
    {
      cursor: 'ready:page-2',
      owner: 'scope',
      repo: 'vcs',
      search: 'atomic refs',
      section: 'ready',
    },
  )
})

test('parseLoadRequestQueueInput rejects unknown sections and searchable private work', () => {
  assert.throws(
    () =>
      parseLoadRequestQueueInput({
        owner: 'scope',
        repo: 'vcs',
        section: 'everything',
      }),
    /section is invalid/,
  )
  assert.throws(
    () =>
      parseLoadRequestQueueInput({
        owner: 'scope',
        repo: 'vcs',
        search: 'private title',
        section: 'your_work',
      }),
    /cannot be searched/,
  )
})

test('parseLoadRequestQueueInput removes empty optional values', () => {
  assert.deepEqual(
    parseLoadRequestQueueInput({
      cursor: ' ',
      owner: 'scope',
      repo: 'vcs',
      search: '\n',
      section: 'completed',
    }),
    {
      cursor: null,
      owner: 'scope',
      repo: 'vcs',
      search: null,
      section: 'completed',
    },
  )
})
