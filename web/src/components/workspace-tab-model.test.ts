import assert from 'node:assert/strict'
import test from 'node:test'
import {
  closeWorkspaceTab,
  emptyWorkspaceTabState,
  openWorkspaceTab,
  pruneWorkspaceTabs,
  workspaceTabDomIds,
  workspaceTabPanelId,
  workspaceTabVisibleLabels,
} from './workspace-tab-model'

test('creates stable linked tab and panel ids for repository paths', () => {
  assert.deepEqual(workspaceTabDomIds('code-files', 'src/hello world.ts'), {
    panelId: 'code-files-panel',
    tabId: 'code-files-tab-src%2Fhello%20world.ts',
  })
  assert.equal(workspaceTabPanelId('code-files'), 'code-files-panel')
  assert.equal(
    workspaceTabDomIds('code-files', 'README.md').panelId,
    workspaceTabDomIds('code-files', 'src/app.ts').panelId,
  )
})

test('previewing a file reuses the preview slot instead of stacking tabs', () => {
  const first = openWorkspaceTab(emptyWorkspaceTabState, 'a', false)
  const second = openWorkspaceTab(first, 'b', false)
  assert.deepEqual(second, { openIds: ['b'], previewId: 'b' })

  const pinned = openWorkspaceTab(second, 'b', true)
  const third = openWorkspaceTab(pinned, 'c', false)
  assert.deepEqual(third, { openIds: ['b', 'c'], previewId: 'c' })
})

test('the preview slot keeps its position when it is replaced', () => {
  const state = openWorkspaceTab(
    openWorkspaceTab(openWorkspaceTab(emptyWorkspaceTabState, 'a', true), 'b', false),
    'c',
    true,
  )
  assert.deepEqual(state, { openIds: ['a', 'c'], previewId: null })
})

test('opening an already open file only promotes it out of preview', () => {
  const preview = openWorkspaceTab(emptyWorkspaceTabState, 'a', false)
  assert.equal(openWorkspaceTab(preview, 'a', false), preview)
  assert.deepEqual(openWorkspaceTab(preview, 'a', true), {
    openIds: ['a'],
    previewId: null,
  })
})

test('closing the active tab selects its right neighbor then its left neighbor', () => {
  assert.deepEqual(
    closeWorkspaceTab({ openIds: ['a', 'b', 'c'], previewId: null }, 'b', 'b'),
    { activeId: 'c', focusId: 'c', state: { openIds: ['a', 'c'], previewId: null } },
  )
  assert.deepEqual(
    closeWorkspaceTab({ openIds: ['a', 'b'], previewId: 'b' }, 'b', 'b'),
    { activeId: 'a', focusId: 'a', state: { openIds: ['a'], previewId: null } },
  )
})

test('closing an inactive or final tab preserves deterministic selection', () => {
  assert.deepEqual(
    closeWorkspaceTab({ openIds: ['a', 'b'], previewId: null }, 'a', 'b'),
    { activeId: 'a', focusId: 'a', state: { openIds: ['a'], previewId: null } },
  )
  assert.deepEqual(
    closeWorkspaceTab({ openIds: ['a'], previewId: null }, 'a', 'a'),
    { activeId: null, focusId: null, state: { openIds: [], previewId: null } },
  )
})

test('pruning drops files that left the projection without reopening any', () => {
  assert.deepEqual(
    pruneWorkspaceTabs(
      { openIds: ['a', 'gone'], previewId: 'gone' },
      new Set(['a']),
    ),
    { openIds: ['a'], previewId: null },
  )
  const unchanged = { openIds: ['a'], previewId: 'a' }
  assert.equal(pruneWorkspaceTabs(unchanged, new Set(['a', 'b'])), unchanged)
})

test('duplicate filenames grow by one parent segment until they are unique', () => {
  assert.deepEqual(
    [...workspaceTabVisibleLabels([
      { id: 'docs/guide.md', label: 'guide.md', title: 'docs/guide.md' },
      { id: 'blog/guide.md', label: 'guide.md', title: 'blog/guide.md' },
      { id: 'README.md', label: 'README.md', title: 'README.md' },
    ])],
    [
      ['docs/guide.md', 'docs/guide.md'],
      ['blog/guide.md', 'blog/guide.md'],
      ['README.md', 'README.md'],
    ],
  )
  assert.deepEqual(
    [...workspaceTabVisibleLabels([
      { id: 'web/src/index.ts', label: 'index.ts', title: 'web/src/index.ts' },
      { id: 'api/src/index.ts', label: 'index.ts', title: 'api/src/index.ts' },
    ])],
    [
      ['web/src/index.ts', 'web/src/index.ts'],
      ['api/src/index.ts', 'api/src/index.ts'],
    ],
  )
})

test('identical trailing segments fall back to the whole path', () => {
  assert.deepEqual(
    [...workspaceTabVisibleLabels([
      { id: 'a/src/index.ts', label: 'index.ts', title: 'a/src/index.ts' },
      { id: 'b/a/src/index.ts', label: 'index.ts', title: 'b/a/src/index.ts' },
    ])],
    [
      ['a/src/index.ts', 'a/src/index.ts'],
      ['b/a/src/index.ts', 'b/a/src/index.ts'],
    ],
  )
})
