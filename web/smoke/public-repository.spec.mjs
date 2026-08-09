import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'

const baseUrl = (
  process.env.SCOPE_WEB_BASE_URL ??
  process.env.PLAYWRIGHT_BASE_URL ??
  'http://localhost:3000'
).replace(/\/$/, '')
const repoId = process.env.SCOPE_SMOKE_REPO ?? process.env.UI_AUDIT_REPO ?? 'dev/public-demo'
const [owner, repo, extra] = repoId.split('/')

if (!owner || !repo || extra) {
  throw new Error('SCOPE_SMOKE_REPO must be an owner/repository pair')
}

const repoPath = `/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`

test('public repository exposes only its projected source', async () => {
  await withPage(repoPath, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Repository' }).waitFor()
    await assertCurrentRepoSection(page, 'Code')
    await page.getByText('2 files', { exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.md' }).waitFor()
    await page.getByRole('button', { name: 'README.md', exact: true }).waitFor()
    assert.equal(await page.getByText('internal', { exact: true }).count(), 0)
    assert.equal(await page.getByText('plan.md', { exact: true }).count(), 0)
    assert.equal(
      await page
        .getByRole('navigation', { name: 'Primary' })
        .getByRole('link', { name: 'Runs', exact: true })
        .count(),
      0,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Recent runs' }).count(),
      0,
    )
    assert.equal(await page.getByRole('heading', { name: 'Runners' }).count(), 0)
  })
})

test('public direct Runs access is explicit and exposes no operations', async () => {
  await withPage(`${repoPath}/runs`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Runs' }).waitFor()
    await page.getByText(
      'Sign in as the owner or a repository member to view runs.',
      { exact: true },
    ).waitFor()
    assert.equal(
      await page
        .getByRole('navigation', { name: 'Primary' })
        .getByRole('link', { name: 'Runs', exact: true })
        .count(),
      0,
    )
    assert.equal(
      await page.getByRole('heading', { name: 'Recent runs' }).count(),
      0,
    )
    assert.equal(await page.getByRole('heading', { name: 'Runners' }).count(), 0)
  })
})

test('public repository history renders its seeded commit', async () => {
  await withPage(`${repoPath}/history`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'History' }).waitFor()
    await assertCurrentRepoSection(page, 'History')
    await page.getByRole('heading', { level: 2, name: 'Commits' }).waitFor()
    const commit = page.getByRole('button', {
      name: 'Projected public update, commit dev-public-1, 2 files',
    })
    await commit.waitFor()
    assert.equal(await commit.getAttribute('title'), 'dev-public-1')
    await commit.getByText('dev-public-1', { exact: true }).waitFor()
    await page.waitForFunction(() => {
      const button = document.querySelector(
        'button[aria-label="Projected public update, commit dev-public-1, 2 files"]',
      )
      return button && Object.keys(button).some((key) => key.startsWith('__reactProps$'))
    })
    await commit.click()
    await page.waitForURL((url) =>
      url.searchParams.get('commit') === 'pv_public_dev-public-1_1'
    )
    assert.equal(
      new URL(page.url()).searchParams.get('commit'),
      'pv_public_dev-public-1_1',
    )
  })
})

test('public repository navigates to history after client hydration', async () => {
  await withPage(repoPath, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Repository' }).waitFor()
    await page.waitForFunction(() => {
      const link = document.querySelector('a[href$="/history"]')
      return link && Object.keys(link).some((key) => key.startsWith('__reactProps$'))
    })
    const documentSentinel = 'scope-history-client-navigation'
    await page.evaluate((sentinel) => {
      window.__scopeSmokeDocument = sentinel
    }, documentSentinel)
    await page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'History', exact: true })
      .click()
    await page.getByRole('heading', { level: 1, name: 'History' }).waitFor()
    await page.getByText('Projected public update', { exact: true }).first().waitFor()
    assert.equal(
      await page.evaluate(() => window.__scopeSmokeDocument),
      documentSentinel,
    )
  })
})

test('repository chrome persists across navigation and request revalidation', async () => {
  await withPage(`/${owner}/update-demo`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Repository' }).waitFor()
    await page.waitForFunction(() => {
      const link = document.querySelector('a[href$="/requests"]')
      return link && Object.keys(link).some((key) => key.startsWith('__reactProps$'))
    })
    const header = await page.locator('header.sticky').elementHandle()
    const navigation = await page
      .getByRole('navigation', { name: 'Primary' })
      .elementHandle()
    assert(header)
    assert(navigation)

    const primaryNavigation = page.getByRole('navigation', { name: 'Primary' })
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'Requests')

    await page
      .getByRole('link', { name: /Add bounded retry timing/ })
      .click()
    await page
      .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
      .waitFor()
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'Requests')

    await page.evaluate(() => globalThis.__TSR_ROUTER__.invalidate())
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'Requests')

    await page.locator('#main-content').evaluate(async (element) => {
      await new Promise((resolve) => {
        element.addEventListener('scroll', resolve, { once: true })
        element.scrollTop = 500
      })
    })
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()
    await page.waitForFunction(
      () => document.querySelector('#main-content')?.scrollTop === 0,
    )
    await assertRepositoryChromePreserved(page, { header, navigation })

    await page.goBack()
    await page
      .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
      .waitFor()
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'Requests')

    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()

    await armNextRouterLoad(page)
    await primaryNavigation
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await page.waitForFunction(() => globalThis.__scopeRouterLoaded === true)
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'Requests')

    await primaryNavigation
      .getByRole('link', { name: 'History', exact: true })
      .click()
    await page.getByRole('heading', { level: 1, name: 'History' }).waitFor()
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'History')
  })
})

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()
    await assertCurrentRepoSection(page, 'Requests')
    await page.getByRole('heading', { level: 2, name: 'Your work' }).waitFor()
    await page.getByRole('heading', { level: 2, name: 'Open' }).waitFor()
    await page.getByRole('heading', { level: 2, name: 'Closed' }).waitFor()
    await page.getByText('No open requests are visible.', { exact: true }).waitFor()
    await page.getByText('No closed requests are visible.', { exact: true }).waitFor()
  })
})

test('seeded request discussion and changes stay reciprocal and ordered', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Add bounded retry timing' }).waitFor()
    const threads = page.locator('.request-discussion-thread')
    await threads.first().waitFor()
    assert.deepEqual(
      await threads.evaluateAll((elements) => elements.map(({ id }) => id)),
      [
        'discussion-discussion_demo_retry_cap',
        'discussion-discussion_demo_jitter',
        'discussion-discussion_demo_resolved_docs',
        'discussion-discussion_demo_revision_jitter',
        'discussion-discussion_demo_revision_tests',
        'discussion-discussion_demo_revision_final',
      ],
    )
    assert.equal(await page.getByRole('textbox').count(), 0)

    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    const expandReplies = retryThread.getByRole('button', { name: '3 replies' })
    await expandReplies.waitFor()
    await page.waitForFunction(
      (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
      await expandReplies.elementHandle(),
    )
    await expandReplies.click()
    const maintainerReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_maintainer',
    )
    await maintainerReply.getByText('Two seconds is intentional', { exact: false }).waitFor()
    await maintainerReply.getByRole('button', { name: 'Show 1 reply' }).click()
    const contributorReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_quote',
    )
    await contributorReply.getByText('Agreed. Quoting the maintainer', { exact: false }).waitFor()
    await contributorReply.getByRole('button', { name: 'Show 1 reply' }).click()
    await page
      .locator('#reply-discussion_reply_demo_retry_cap_nested')
      .getByText('Exactly. Keeping that decision nested', { exact: false })
      .waitFor()

    const anchoredThread = page.locator(
      '#discussion-discussion_demo_revision_jitter',
    )
    await anchoredThread.getByRole('link', { name: /Revision/ }).click()
    await page.waitForURL((url) => (
      url.pathname.endsWith('/requests/req_demo_ready/changes') &&
      url.searchParams.get('revision') === 'event_req_demo_ready_revision_2'
    ))
    assert.equal(await page.getByRole('textbox').count(), 0)
    await page
      .getByRole('link', { name: /The bounded jitter looks right/ })
      .click()
    await page.waitForURL((url) => (
      url.pathname.endsWith('/requests/req_demo_ready') &&
      url.searchParams.get('discussion') === 'discussion_demo_revision_jitter'
    ))
    await page.locator('.request-discussion-thread').first().waitFor()

    const requestViews = page.getByRole('navigation', { name: 'Request views' })
    const changesLink = requestViews.getByRole('link', { name: 'Changes' })
    await page.waitForFunction(
      (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
      await changesLink.elementHandle(),
    )
    const requestHeading = await page
      .getByRole('heading', { level: 1, name: 'Add bounded retry timing' })
      .elementHandle()
    const requestNavigation = await requestViews.elementHandle()
    assert(requestHeading)
    assert(requestNavigation)
    await page.locator('#main-content').evaluate((element) => {
      element.scrollTop = 200
    })
    await page.waitForFunction(
      () => document.querySelector('#main-content')?.scrollTop === 200,
    )
    const requestScroll = await page.locator('#main-content').evaluate(
      (element) => element.scrollTop,
    )
    assert(requestScroll > 0)

    await changesLink.click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready/changes'))
    await page.getByRole('heading', { level: 2, name: 'Commits' }).waitFor()
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
      scroll: requestScroll,
    })
    await page.getByRole('navigation', { name: 'Request views' })
      .getByRole('link', { name: 'Discussion' })
      .click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready'))
    await page.locator('.request-discussion-thread').first().waitFor()
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
      scroll: requestScroll,
    })
  })
})

test('request queue search is keyboard accessible and mobile rows do not overflow', async () => {
  await withPage(
    `/${owner}/update-demo/requests`,
    async (page) => {
      const readyRow = page.getByRole('link', {
        name: /Add bounded retry timing/,
      })
      await readyRow.waitFor()
      await readyRow.focus()
      assert.equal(
        await readyRow.evaluate(
          (element) => element === document.activeElement,
        ),
        true,
      )
      const search = page.getByRole('searchbox', {
        name: 'Search open and closed requests',
      })
      const queueRequests = []
      page.on('request', (request) => {
        if (request.url().includes('/_serverFn/')) {
          queueRequests.push(request.url())
        }
      })
      await page.waitForFunction(
        (element) =>
          Object.keys(element).some((key) => key.startsWith('__reactProps$')),
        await search.elementHandle(),
      )
      await search.fill('missing request title')
      await search.press('Enter')
      await page
        .getByText(
          'No open requests match “missing request title”.',
          { exact: true },
        )
        .waitFor()
      assert.equal(queueRequests.length, 2)
      const clear = page.getByRole('button', { name: 'Clear' })
      await clear.focus()
      assert.equal(
        await clear.evaluate((element) => element === document.activeElement),
        true,
      )
      assert.equal(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
            document.documentElement.clientWidth,
        ),
        true,
      )
    },
    { viewport: { height: 844, width: 390 } },
  )
})

async function withPage(path, assertion, pageOptions = {}) {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage(pageOptions)
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))

  try {
    const response = await page.goto(new URL(path, `${baseUrl}/`).toString(), {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })
    assert(response, `navigation to ${path} did not produce a response`)
    assert(response.status() < 400, `navigation to ${path} returned ${response.status()}`)
    await assertion(page)
    assert.deepEqual(pageErrors, [])
  } finally {
    await browser.close()
  }
}

async function assertCurrentRepoSection(page, section) {
  const link = page
    .getByRole('navigation', { name: 'Primary' })
    .getByRole('link', { name: section, exact: true })
  await link.waitFor()
  assert.equal(await link.getAttribute('aria-current'), 'page')
  await page.waitForFunction(
    (expected) => {
      const current = [
        ...document.querySelectorAll(
          'nav[aria-label="Primary"] a[aria-current="page"]',
        ),
      ].map((link) => link.textContent?.trim())
      return current.length === 1 && current[0] === expected
    },
    section,
  )
}

async function assertRepositoryChromePreserved(page, chrome) {
  assert.equal(
    await page.evaluate(
      ({ header, navigation }) =>
        header === document.querySelector('header.sticky') &&
        navigation === document.querySelector('nav[aria-label="Primary"]'),
      chrome,
    ),
    true,
  )
}

async function assertRequestShellPreserved(page, shell) {
  assert.equal(
    await page.evaluate(
      ({ heading, navigation }) =>
        heading === document.querySelector('h1') &&
        navigation === document.querySelector('nav[aria-label="Request views"]'),
      shell,
    ),
    true,
  )
  assert.equal(
    await page.locator('#main-content').evaluate(
      (element) => element.scrollTop,
    ),
    shell.scroll,
  )
}

async function armNextRouterLoad(page) {
  await page.evaluate(() => {
    globalThis.__scopeRouterLoaded = false
    const unsubscribe = globalThis.__TSR_ROUTER__.subscribe(
      'onLoad',
      () => {
        unsubscribe()
        globalThis.__scopeRouterLoaded = true
      },
    )
  })
}
