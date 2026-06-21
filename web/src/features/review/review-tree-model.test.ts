import * as assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  buildReviewTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  normalizeReviewPath,
} from './review-tree-model'

type TestFile = {
  path: string
  visibility: 'Private' | 'Public'
}

test('buildReviewTree nests folders, sorts folders before files, and normalizes paths', () => {
  const tree = buildReviewTree<TestFile>([
    { path: '/src/zeta.ts', visibility: 'Public' },
    { path: 'README.md', visibility: 'Public' },
    { path: String.raw`src\components\Button.tsx`, visibility: 'Private' },
    { path: './docs//guide.md', visibility: 'Public' },
    { path: '/src/components/Alert.tsx', visibility: 'Private' },
  ])

  assert.deepEqual(
    tree.children.map((node) => `${node.type}:${node.name}`),
    ['folder:docs', 'folder:src', 'file:README.md'],
  )

  const src = tree.children[1]
  assert.equal(src.type, 'folder')
  if (src.type !== 'folder') {
    throw new Error('src should be a folder')
  }

  assert.deepEqual(
    src.children.map((node) => `${node.type}:${node.name}`),
    ['folder:components', 'file:zeta.ts'],
  )
  assert.equal(folderVisibility(src.files), 'Mixed')

  const components = src.children[0]
  assert.equal(components.type, 'folder')
  if (components.type !== 'folder') {
    throw new Error('components should be a folder')
  }

  assert.deepEqual(
    components.children.map((node) => node.path),
    ['src/components/Alert.tsx', 'src/components/Button.tsx'],
  )
  assert.equal(folderVisibility(components.files), 'Private')
})

test('folderVisibility reports Public, Private, and Mixed states', () => {
  assert.equal(folderVisibility([{ path: 'a.ts', visibility: 'Public' }]), 'Public')
  assert.equal(
    folderVisibility([{ path: 'a.ts', visibility: 'Private' }]),
    'Private',
  )
  assert.equal(
    folderVisibility([
      { path: 'a.ts', visibility: 'Private' },
      { path: 'b.ts', visibility: 'Public' },
    ]),
    'Mixed',
  )
})

test('folderCollapseKeys returns every folder key for collapsed review landing', () => {
  const tree = buildReviewTree<TestFile>([
    { path: 'api/src/main.rs', visibility: 'Private' },
    { path: 'api/tests/http.rs', visibility: 'Private' },
    { path: 'web/src/App.tsx', visibility: 'Public' },
    { path: 'README.md', visibility: 'Public' },
  ])

  assert.deepEqual(folderCollapseKeys(tree), [
    'folder:/api',
    'folder:/api/src',
    'folder:/api/tests',
    'folder:/web',
    'folder:/web/src',
  ])
})

test('path helpers normalize leading slashes, backslashes, dot segments, and empty parts', () => {
  assert.equal(
    normalizeReviewPath(String.raw`\src//components/./Button.tsx`),
    'src/components/Button.tsx',
  )
  assert.equal(displayPath('/docs//guide.md'), 'docs/guide.md')
})

