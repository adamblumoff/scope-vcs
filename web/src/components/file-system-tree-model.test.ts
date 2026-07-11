import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  buildFileSystemTree,
  displayPath,
  folderCollapseKeys,
  folderVisibility,
  normalizeFilePath,
} from './file-system-tree-model'

type TestFile = { path: string; visibility: 'Private' | 'Public' }

test('file tree normalizes, nests, sorts, and summarizes paths', () => {
  const tree = buildFileSystemTree<TestFile>([
    { path: '/src/zeta.ts', visibility: 'Public' },
    { path: 'README.md', visibility: 'Public' },
    { path: String.raw`src\components\Button.tsx`, visibility: 'Private' },
    { path: './docs//guide.md', visibility: 'Public' },
    { path: '/src/components/Alert.tsx', visibility: 'Private' },
  ])

  assert.deepEqual(tree.children.map(({ type, name }) => [type, name]), [
    ['folder', 'docs'], ['folder', 'src'], ['file', 'README.md'],
  ])
  const src = tree.children[1]
  assert.equal(src.type, 'folder')
  assert.deepEqual(src.children.map(({ type, name }) => [type, name]), [
    ['folder', 'components'], ['file', 'zeta.ts'],
  ])
  assert.deepEqual(src.files.map(({ path }) => displayPath(path)), [
    'src/components/Alert.tsx', 'src/components/Button.tsx', 'src/zeta.ts',
  ])
  assert.deepEqual(folderCollapseKeys(tree), [
    'folder:/docs', 'folder:/src', 'folder:/src/components',
  ])
  assert.equal(folderVisibility(src.files), 'Mixed')
  assert.equal(folderVisibility([{ path: 'a', visibility: 'Public' }]), 'Public')
  assert.equal(folderVisibility([{ path: 'a', visibility: 'Private' }]), 'Private')
  assert.equal(
    normalizeFilePath(String.raw`.\src\\..\components/Button.tsx`),
    'src/components/Button.tsx',
  )
})
