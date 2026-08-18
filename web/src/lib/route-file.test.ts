import assert from 'node:assert/strict'
import test from 'node:test'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
  selectedRouteFilePath,
} from './route-file'

test('normalizes repository file paths for URLs', () => {
  assert.equal(displayRouteFilePath('/README.html'), 'README.html')
  assert.equal(parseRouteFileSearch('/docs/guide.md'), 'docs/guide.md')
})

test('rejects empty, non-string, and traversing route file searches', () => {
  for (const value of ['', null, 42, '.', '..', '../secret', 'src/../secret']) {
    assert.equal(parseRouteFileSearch(value), undefined)
  }
})

test('resolves a normalized route path against repository files', () => {
  const files = [{ path: '/README.html' }, { path: 'src/app.ts' }]

  assert.equal(selectedRouteFilePath(files, 'README.html'), '/README.html')
  assert.equal(selectedRouteFilePath(files, 'src/app.ts'), 'src/app.ts')
  assert.equal(selectedRouteFilePath(files, 'missing.ts'), null)
})
