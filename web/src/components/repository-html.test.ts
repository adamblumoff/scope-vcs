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
    'dark',
  )

  assert.match(
    document,
    /^<!doctype html><meta charset="utf-8">/,
  )
  assert.match(document, /<meta http-equiv="Content-Security-Policy"/)
  assert.match(
    document,
    /<base target="_blank"><style>:root\{color-scheme:dark!important\}<\/style><html><head><title>Project<\/title>/,
  )
  assert.match(document, /<body>Hello<\/body>/)
})

test('enforces the policy when authored markup contains misleading head text', () => {
  const withRoot = repositoryHtmlDocument(
    '<!-- <head> --><html lang="en"><body>Hello</body></html>',
    'light',
  )
  const fragment = repositoryHtmlDocument('<main>Hello</main>', 'light')

  assert.match(
    withRoot,
    /<style>:root\{color-scheme:light!important\}<\/style><!-- <head> --><html lang="en">/,
  )
  assert.match(
    fragment,
    /<style>:root\{color-scheme:light!important\}<\/style><main>/,
  )
})

test('removes authored meta elements that could navigate the preview', () => {
  const document = repositoryHtmlDocument(
    `
      <html>
        <head>
          <META HTTP-EQUIV="refresh" content="0 > 0;url=https://example.com">
          <meta/name="theme-color" content="red">
          <metadata>kept</metadata>
          <style>body { color: green; }</style>
        </head>
        <body>Hello</body>
      </html>
    `,
    'dark',
  )

  assert.doesNotMatch(document, /<meta http-equiv="refresh"/i)
  assert.doesNotMatch(document, /0 > 0;url=https:\/\/example.com/)
  assert.doesNotMatch(document, /name="theme-color"/)
  assert.match(document, /<metadata>kept<\/metadata>/)
  assert.match(document, /<style>body \{ color: green; \}<\/style>/)
  assert.match(document, /<meta charset="utf-8">/)
  assert.match(
    document,
    /<meta name="viewport" content="width=device-width, initial-scale=1">/,
  )
  assert.match(
    document,
    /<meta http-equiv="x-dns-prefetch-control" content="off">/,
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
