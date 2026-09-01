import assert from 'node:assert/strict'

export async function assertRequestCrossLinksStayInDocument(page) {
  const requestViews = page.getByRole('navigation', { name: 'Request views' })
  const heading = await page
    .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
    .elementHandle()
  const navigation = await requestViews.elementHandle()
  assert(heading)
  assert(navigation)
  const documentSentinel = 'scope-request-cross-navigation'
  await page.evaluate((sentinel) => {
    window.__scopeRequestDocument = sentinel
  }, documentSentinel)

  const anchoredThread = page.locator(
    '#discussion-discussion_demo_revision_jitter',
  )
  const revisionLink = anchoredThread.getByRole('link', { name: /Revision/ })
  await waitForClientHydration(page, revisionLink)
  await revisionLink.click()
  await page.waitForURL((url) => (
    url.pathname.endsWith('/requests/req_demo_ready/changes') &&
    url.searchParams.get('revision') === 'event_req_demo_ready_revision_2'
  ))
  await assertRequestDocumentAndShell(page, { documentSentinel, heading, navigation })
  assert.equal(await page.getByRole('textbox').count(), 0)

  const discussionLink = page.getByRole('link', {
    name: /The bounded jitter looks right/,
  })
  await waitForClientHydration(page, discussionLink)
  await discussionLink.click()
  await page.waitForURL((url) => (
    url.pathname.endsWith('/requests/req_demo_ready') &&
    url.searchParams.get('discussion') === 'discussion_demo_revision_jitter' &&
    url.hash === '#discussion-discussion_demo_revision_jitter'
  ))
  await page.locator('.request-discussion-thread').first().waitFor()
  await assertRequestDocumentAndShell(page, { documentSentinel, heading, navigation })
  return { heading, navigation }
}

export async function waitForClientHydration(page, locator) {
  const element = await locator.elementHandle()
  assert(element)
  await page.waitForFunction(
    (target) => Object.keys(target).some((key) => key.startsWith('__reactProps$')),
    element,
  )
}

async function assertRequestDocumentAndShell(page, shell) {
  assert.equal(
    await page.evaluate(() => window.__scopeRequestDocument),
    shell.documentSentinel,
  )
  await assertRequestShellPreserved(page, shell)
}

export async function assertFileSelectionSkipsRevisionReload(page, fileName, path) {
  await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const fileNavigator = page.getByLabel('Commit file navigator')
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const id = new URL(request.url()).pathname.split('/').at(-1)
      serverFunctions.push(JSON.parse(Buffer.from(id, 'base64url')).export)
    }
  }
  page.on('request', recordServerFunction)
  try {
    for (const folder of path.split('/').filter(Boolean).slice(0, -1)) {
      const expandFolder = fileNavigator.getByRole('button', {
        exact: true,
        name: `Expand ${folder}`,
      })
      if (await expandFolder.count()) await expandFolder.click()
    }
    await fileNavigator.getByRole('button', { name: fileName }).click()
    await page.waitForURL((url) => url.searchParams.get('path') === path)
    const diff = page.getByLabel(`${path.replace(/^\/+/, '')} diff`, { exact: true })
    await diff.waitFor()
    await diff.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.deepEqual(serverFunctions, ['loadRevisionDiff_createServerFn_handler'])
}

export async function assertUpdateSelectionReloadsSelectedPayload(page) {
  await page.locator('[data-slot="pending-surface"]').waitFor({ state: 'detached' })
  const updates = page.getByRole('button', {
    name: /, commit .+, \d+ files?$/,
  })
  assert(await updates.count() > 1, 'expected more than one request update')
  const target = updates.nth(1)
  const commit = await target.getAttribute('title')
  const targetLabel = await target.getAttribute('aria-label')
  assert(commit)
  assert(targetLabel)
  const title = targetLabel.split(', commit ', 1)[0]
  const serverFunctions = []
  const recordServerFunction = (request) => {
    if (request.url().includes('/_serverFn/')) {
      const id = new URL(request.url()).pathname.split('/').at(-1)
      serverFunctions.push(JSON.parse(Buffer.from(id, 'base64url')).export)
    }
  }
  page.on('request', recordServerFunction)
  try {
    await target.click()
    await page.waitForURL((url) => (
      url.searchParams.get('commit') === commit &&
      !url.searchParams.has('path')
    ))
    await page.getByRole('heading', { level: 3, name: title }).waitFor()
  } finally {
    page.off('request', recordServerFunction)
  }
  assert.deepEqual(serverFunctions, ['loadChangesPage_createServerFn_handler'])
}

export async function assertRequestShellPreserved(page, shell) {
  assert.equal(
    await page.evaluate(
      ({ heading, navigation }) =>
        heading === document.querySelector('h1') &&
        navigation === document.querySelector('nav[aria-label="Request views"]'),
      shell,
    ),
    true,
  )
}
