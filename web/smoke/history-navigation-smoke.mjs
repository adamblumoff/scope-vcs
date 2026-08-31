import assert from 'node:assert/strict'

export async function assertHistoryFirstFileStaysInRoute(page, diffWorkers) {
  const fileNavigator = page.getByLabel('Update file navigator')
  await fileNavigator.waitFor()
  await page.waitForFunction(
    (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
    await fileNavigator.elementHandle(),
  )
  await page.waitForFunction(
    () => globalThis.__TSR_ROUTER__.state.status === 'idle',
  )
  const documentSentinel = 'scope-history-file-selection'
  await page.evaluate((sentinel) => {
    window.__scopeHistoryDocument = sentinel
  }, documentSentinel)
  let serverFunctionRequests = 0
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      serverFunctionRequests += 1
    }
  }
  page.on('request', recordServerFunction)
  try {
    await fileNavigator
      .getByRole('button', { name: 'README.html', exact: true })
      .click()
    await page.waitForURL((url) => (
      url.searchParams.get('path') === '/README.html' &&
      !url.searchParams.has('audience')
    ))
    const diff = page.getByLabel('README.html diff', { exact: true })
    await diff.waitFor()
    await diff.locator('[data-slot="pending-surface"]').waitFor({
      state: 'detached',
    })
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.equal(
    await page.evaluate(() => window.__scopeHistoryDocument),
    documentSentinel,
  )
  assert.equal(serverFunctionRequests, 1)
  assert.equal(
    diffWorkers.some((url) => url.includes('worker-portable')),
    true,
    'the diff worker should start before the first file selection',
  )
}
