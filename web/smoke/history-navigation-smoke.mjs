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
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      serverFunctions.push(serverFunctionExport(request))
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
  assert.deepEqual(serverFunctions, [
    'loadHistoryEntryFileDiff_createServerFn_handler',
  ])
  assert.equal(
    diffWorkers.some((url) => url.includes('worker-portable')),
    true,
    'the diff worker should start before the first file selection',
  )
}

function serverFunctionExport(request) {
  const encodedId = new URL(request.url()).pathname.split('/').at(-1)
  assert(encodedId, 'server function request is missing its encoded id')
  return JSON.parse(Buffer.from(encodedId, 'base64url')).export
}
