import assert from 'node:assert/strict'
import test from 'node:test'
import {
  isRepositoryHtmlPath,
  repositoryHtmlDocument,
  REPOSITORY_HTML_CONTENT_SECURITY_POLICY,
} from './repository-html'

test('recognizes HTML documents without treating adjacent formats as HTML', () => {
  for (const path of ['README.html', '/docs/demo.HTML']) {
    assert.equal(isRepositoryHtmlPath(path), true)
  }
  for (const path of ['README.htm', 'component.tsx', 'html']) {
    assert.equal(isRepositoryHtmlPath(path), false)
  }
})

test('prefixes the repository policy ahead of authored markup', () => {
  const document = repositoryHtmlDocument(
    '<!doctype html><html><head><title>Project</title></head><body>Hello</body></html>',
  )

  assert.match(
    document,
    /^<!doctype html><meta http-equiv="Content-Security-Policy"/,
  )
  assert.match(document, /<base target="_blank"><html><head><title>Project<\/title>/)
  assert.match(document, /<body>Hello<\/body>/)
})

test('enforces the policy when authored markup contains misleading head text', () => {
  const withRoot = repositoryHtmlDocument(
    '<!-- <head> --><html lang="en"><body>Hello</body></html>',
  )
  const fragment = repositoryHtmlDocument('<main>Hello</main>')

  assert.match(
    withRoot,
    /^<!doctype html><meta http-equiv="Content-Security-Policy"[^>]+><base target="_blank"><!-- <head> -->/,
  )
  assert.match(
    fragment,
    /^<!doctype html><meta http-equiv="Content-Security-Policy"[^>]+><base target="_blank"><main>/,
  )
})

test('denies script, network, framing, forms, and remote resources', () => {
  for (const directive of [
    "default-src 'none'",
    "connect-src 'none'",
    "script-src 'none'",
    "frame-src 'none'",
    "form-action 'none'",
    "object-src 'none'",
  ]) {
    assert.match(REPOSITORY_HTML_CONTENT_SECURITY_POLICY, new RegExp(directive))
  }
  assert.match(REPOSITORY_HTML_CONTENT_SECURITY_POLICY, /style-src 'unsafe-inline'/)
  assert.match(REPOSITORY_HTML_CONTENT_SECURITY_POLICY, /img-src data:/)
  assert.match(REPOSITORY_HTML_CONTENT_SECURITY_POLICY, /font-src data:/)
})
