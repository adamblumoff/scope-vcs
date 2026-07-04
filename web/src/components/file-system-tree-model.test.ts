import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import type { VisibilityState } from '@/api/types'
import {
  buildFileSystemTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  normalizeFilePath,
} from './file-system-tree-model'

type TestFile = {
  path: string
  visibility: 'Private' | 'Public'
}

test('buildFileSystemTree nests folders, sorts folders before files, and normalizes paths', () => {
  const tree = buildFileSystemTree<TestFile>([
    { path: '/src/zeta.ts', visibility: 'Public' },
    { path: 'README.md', visibility: 'Public' },
    { path: String.raw`src\components\Button.tsx`, visibility: 'Private' },
    { path: './docs//guide.md', visibility: 'Public' },
    { path: '/src/components/Alert.tsx', visibility: 'Private' },
  ])

  assert.deepEqual(
    tree.children.map((node) => [node.type, node.name]),
    [
      ['folder', 'docs'],
      ['folder', 'src'],
      ['file', 'README.md'],
    ],
  )

  const src = tree.children[1]
  assert.equal(src.type, 'folder')
  assert.deepEqual(
    src.children.map((node) => [node.type, node.name]),
    [
      ['folder', 'components'],
      ['file', 'zeta.ts'],
    ],
  )
  assert.deepEqual(
    src.files.map((file) => displayPath(file.path)),
    [
      'src/components/Alert.tsx',
      'src/components/Button.tsx',
      'src/zeta.ts',
    ],
  )

  const components = src.children[0]
  assert.equal(components.type, 'folder')
  assert.deepEqual(
    components.children.map((node) => node.name),
    ['Alert.tsx', 'Button.tsx'],
  )
})

test('folderVisibility summarizes descendant files', () => {
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
    'Mixed' satisfies VisibilityState,
  )
})

test('folderCollapseKeys returns nested folder keys and skips files', () => {
  const tree = buildFileSystemTree<TestFile>([
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

test('normalizeFilePath removes traversal, duplicate separators, and platform separators', () => {
  assert.equal(
    normalizeFilePath(String.raw`.\src\\..\components/Button.tsx`),
    'src/components/Button.tsx',
  )
})
