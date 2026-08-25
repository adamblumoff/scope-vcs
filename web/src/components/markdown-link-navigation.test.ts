import assert from 'node:assert/strict'
import test from 'node:test'
import {
  markdownClientNavigationHref,
  type MarkdownLinkClick,
} from './markdown-link-navigation'

const currentHref = 'https://scope.test/acme/widgets/requests/42'
const plainClick: MarkdownLinkClick = {
  altKey: false,
  button: 0,
  ctrlKey: false,
  defaultPrevented: false,
  href: '/acme/widgets/requests/43?view=changes#diff',
  metaKey: false,
  shiftKey: false,
}
const isClientRoute = (pathname: string) => pathname.startsWith('/acme/')

test('resolves app-relative and same-origin links for client navigation', () => {
  assert.equal(
    markdownClientNavigationHref(plainClick, currentHref, isClientRoute),
    '/acme/widgets/requests/43?view=changes#diff',
  )
  assert.equal(
    markdownClientNavigationHref(
      { ...plainClick, href: './41?discussion=one#discussion-one' },
      currentHref,
      isClientRoute,
    ),
    '/acme/widgets/requests/41?discussion=one#discussion-one',
  )
  assert.equal(
    markdownClientNavigationHref(
      { ...plainClick, href: 'https://scope.test/acme/widgets' },
      currentHref,
      isClientRoute,
    ),
    '/acme/widgets',
  )
})

test('leaves external URLs, fragments, and other protocols to the browser', () => {
  for (const href of [
    'https://example.com/acme/widgets',
    '#discussion-one',
    '/acme/widgets/requests/42#discussion-one',
    'https://scope.test/acme/widgets/requests/42#discussion-one',
    'mailto:hello@example.com',
    'tel:+15555550123',
    'custom:payload',
  ]) {
    assert.equal(
      markdownClientNavigationHref(
        { ...plainClick, href },
        currentHref,
        isClientRoute,
      ),
      null,
    )
  }
})

test('leaves same-origin server resources outside the route tree to the browser', () => {
  for (const href of ['/v1/repos', '/assets/archive.patch']) {
    assert.equal(
      markdownClientNavigationHref(
        { ...plainClick, href },
        currentHref,
        isClientRoute,
      ),
      null,
    )
  }
})

test('leaves nonstandard clicks and links with native attributes to the browser', () => {
  for (const overrides of [
    { altKey: true },
    { button: 1 },
    { ctrlKey: true },
    { defaultPrevented: true },
    { download: true },
    { download: 'archive.patch' },
    { metaKey: true },
    { shiftKey: true },
    { target: '_blank' },
    { target: '_self' },
  ]) {
    assert.equal(
      markdownClientNavigationHref(
        { ...plainClick, ...overrides },
        currentHref,
        isClientRoute,
      ),
      null,
    )
  }
})
