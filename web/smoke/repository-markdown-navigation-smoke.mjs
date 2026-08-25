import assert from 'node:assert/strict'

export async function assertRepositoryMarkdownUsesClientNavigation(page, owner) {
  await page.getByRole('button', { name: 'README.md', exact: true }).click()
  await page.getByRole('heading', { level: 1, name: 'Update Demo' }).waitFor()
  const header = await page.locator('header.sticky').elementHandle()
  const navigation = await page
    .getByRole('navigation', { name: 'Primary' })
    .elementHandle()
  assert(header)
  assert(navigation)
  const documentSentinel = 'scope-repository-markdown-navigation'
  await page.evaluate((sentinel) => {
    window.__scopeMarkdownDocument = sentinel
  }, documentSentinel)

  await page.getByRole('link', { name: 'Read the release guide' }).click()
  await page.waitForURL((url) => (
    url.pathname === `/${owner}/update-demo` &&
    url.searchParams.get('file') === 'docs/release.md'
  ))
  await page.getByRole('heading', { level: 1, name: 'Release flow' }).waitFor()
  assert.equal(
    await page.evaluate(() => window.__scopeMarkdownDocument),
    documentSentinel,
  )
  assert.equal(
    await page.evaluate(
      ({ header, navigation }) =>
        header === document.querySelector('header.sticky') &&
        navigation === document.querySelector('nav[aria-label="Primary"]'),
      { header, navigation },
    ),
    true,
  )
}
