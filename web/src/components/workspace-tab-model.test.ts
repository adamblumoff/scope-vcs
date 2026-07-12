import assert from 'node:assert/strict'
import test from 'node:test'
import {
  closeWorkspaceTab,
  normalizeWorkspaceTabIds,
  workspaceTabVisibleLabels,
} from './workspace-tab-model'

test('normalizes tabs against available items and appends the active item once', () => {
  assert.deepEqual(
    normalizeWorkspaceTabIds(
      ['README.md', 'missing.ts', 'README.md'],
      new Set(['README.md', 'src/app.ts']),
      'src/app.ts',
    ),
    ['README.md', 'src/app.ts'],
  )
})

test('closing the active tab selects its right neighbor then its left neighbor', () => {
  assert.deepEqual(
    closeWorkspaceTab(['a', 'b', 'c'], 'b', 'b'),
    { activeId: 'c', focusId: 'c', openIds: ['a', 'c'] },
  )
  assert.deepEqual(
    closeWorkspaceTab(['a', 'b'], 'b', 'b'),
    { activeId: 'a', focusId: 'a', openIds: ['a'] },
  )
})

test('closing an inactive or final tab preserves deterministic selection', () => {
  assert.deepEqual(
    closeWorkspaceTab(['a', 'b'], 'a', 'b'),
    { activeId: 'a', focusId: 'a', openIds: ['a'] },
  )
  assert.deepEqual(
    closeWorkspaceTab(['a'], 'a', 'a'),
    { activeId: null, focusId: null, openIds: [] },
  )
})

test('duplicate basenames are visually disambiguated with their full paths', () => {
  assert.deepEqual(
    [...workspaceTabVisibleLabels([
      { id: 'src/index.ts', label: 'index.ts', title: 'src/index.ts' },
      { id: 'tests/index.ts', label: 'index.ts', title: 'tests/index.ts' },
      { id: 'README.md', label: 'README.md', title: 'README.md' },
    ])],
    [
      ['src/index.ts', 'src/index.ts'],
      ['tests/index.ts', 'tests/index.ts'],
      ['README.md', 'README.md'],
    ],
  )
})
