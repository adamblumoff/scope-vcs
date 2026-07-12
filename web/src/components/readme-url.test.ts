import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveReadmeUrl, safeMarkdownUrl } from './readme-url'

const context = {
  owner: 'scope',
  readmePath: 'docs/README.md',
  repo: 'demo',
}

test('allows anchors and approved README link schemes', () => {
  for (const url of [
    'https://example.com',
    'http://example.com',
    'mailto:hello@example.com',
  ]) {
    assert.equal(safeMarkdownUrl(url), url)
  }
  assert.equal(safeMarkdownUrl('#installation'), '#readme-installation')
  assert.equal(safeMarkdownUrl('#Getting-Started'), '#readme-getting-started')
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

test('resolves relative README links to repository file routes', () => {
  assert.equal(
    resolveReadmeUrl('./guide.md#usage', context),
    '/repos/scope/demo?file=docs%2Fguide.md#readme-usage',
  )
  assert.equal(
    resolveReadmeUrl('../LICENSE', context),
    '/repos/scope/demo?file=LICENSE',
  )
  assert.equal(
    resolveReadmeUrl('/CONTRIBUTING.md', context),
    '/repos/scope/demo?file=CONTRIBUTING.md',
  )
  assert.equal(
    resolveReadmeUrl('My%20Guide.md', context),
    '/repos/scope/demo?file=docs%2FMy%20Guide.md',
  )
})

test('rejects relative README paths that escape the repository', () => {
  assert.equal(resolveReadmeUrl('../../LICENSE', context), '')
  assert.equal(resolveReadmeUrl('%2e%2e/%2e%2e/LICENSE', context), '')
  assert.equal(resolveReadmeUrl('bad%escape.md', context), '')
  assert.equal(resolveReadmeUrl('//example.com/tracker.png', context), '')
  for (const url of [
    'javascript:alert(1)',
    'data:text/html,hello',
    'file:///etc/passwd',
    'custom:payload',
  ]) {
    assert.equal(resolveReadmeUrl(url, context), '')
  }
})
