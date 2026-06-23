import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  parseSetRepoFileVisibilityInput,
  parseUpdateRepoSettingsInput,
} from './repo-inputs'

test('parseSetRepoFileVisibilityInput trims repo params and file paths', () => {
  assert.deepEqual(
    parseSetRepoFileVisibilityInput({
      owner: ' adam ',
      paths: [' /README.md ', '', 42, '/src/app.ts'],
      repo: ' scope ',
      visibility: 'Public',
    }),
    {
      owner: 'adam',
      paths: ['/README.md', '/src/app.ts'],
      repo: 'scope',
      visibility: 'Public',
    },
  )
})

test('parseSetRepoFileVisibilityInput requires at least one file path', () => {
  assert.throws(
    () =>
      parseSetRepoFileVisibilityInput({
        owner: 'adam',
        paths: [],
        repo: 'scope',
        visibility: 'Private',
      }),
    /At least one file path is required/,
  )
})

test('parseUpdateRepoSettingsInput normalizes repo settings', () => {
  assert.deepEqual(
    parseUpdateRepoSettingsInput({
      default_new_file_visibility: 'Public',
      owner: ' adam ',
      repo: ' scope ',
      review_pushes_before_applying: false,
    }),
    {
      default_new_file_visibility: 'Public',
      owner: 'adam',
      repo: 'scope',
      review_pushes_before_applying: false,
    },
  )
})
