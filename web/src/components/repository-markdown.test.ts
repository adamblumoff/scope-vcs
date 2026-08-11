import assert from 'node:assert/strict'
import test from 'node:test'
import {
  isRepositoryMarkdownPath,
  resolveRepositoryMarkdownUrl,
  safeMarkdownUrl,
} from './repository-markdown'

const context = {
  markdownPath: 'docs/guide.md',
  owner: 'scope',
  repo: 'demo',
}

test('recognizes Markdown documents anywhere in the repository', () => {
  for (const path of ['README.md', 'docs/guide.md', '/notes/PLAN.MD']) {
    assert.equal(isRepositoryMarkdownPath(path), true)
  }
  for (const path of [
    'README',
    'README.txt',
    'docs/guide.markdown',
    'docs/guide.mdx',
    'src/md',
  ]) {
    assert.equal(isRepositoryMarkdownPath(path), false)
  }
})

test('allows anchors and approved Markdown link schemes', () => {
  for (const url of [
    'https://example.com',
    'http://example.com',
    'mailto:hello@example.com',
  ]) {
    assert.equal(safeMarkdownUrl(url), url)
  }
  assert.equal(safeMarkdownUrl('#installation'), '#markdown-installation')
  assert.equal(safeMarkdownUrl('#Getting-Started'), '#markdown-getting-started')
  assert.equal(safeMarkdownUrl('#user-content-fn-1'), '#user-content-fn-1')
  assert.equal(safeMarkdownUrl('#user-content-fnref-1'), '#user-content-fnref-1')
})

test('rejects unresolved repository paths and unsafe URL schemes', () => {
  for (const url of [
    './docs/guide.md',
    '/absolute/path',
    '//example.com/tracker.png',
    'javascript:alert(1)',
    'data:text/html,hello',
    'file:///etc/passwd',
    'vbscript:msgbox(1)',
  ]) {
    assert.equal(safeMarkdownUrl(url), '')
  }
})

test('resolves relative Markdown links to repository file routes', () => {
  assert.equal(
    resolveRepositoryMarkdownUrl('./setup.md#usage', context),
    '/scope/demo?file=docs%2Fsetup.md#markdown-usage',
  )
  assert.equal(
    resolveRepositoryMarkdownUrl('../LICENSE', context),
    '/scope/demo?file=LICENSE',
  )
  assert.equal(
    resolveRepositoryMarkdownUrl('/CONTRIBUTING.md', context),
    '/scope/demo?file=CONTRIBUTING.md',
  )
  assert.equal(
    resolveRepositoryMarkdownUrl('My%20Guide.md', context),
    '/scope/demo?file=docs%2FMy%20Guide.md',
  )
})

test('rejects relative Markdown paths that escape the repository', () => {
  assert.equal(resolveRepositoryMarkdownUrl('../../LICENSE', context), '')
  assert.equal(resolveRepositoryMarkdownUrl('%2e%2e/%2e%2e/LICENSE', context), '')
  assert.equal(resolveRepositoryMarkdownUrl('bad%escape.md', context), '')
  assert.equal(resolveRepositoryMarkdownUrl('//example.com/tracker.png', context), '')
  for (const url of [
    'javascript:alert(1)',
    'data:text/html,hello',
    'file:///etc/passwd',
    'custom:payload',
  ]) {
    assert.equal(resolveRepositoryMarkdownUrl(url, context), '')
  }
})
