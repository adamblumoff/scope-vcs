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
    await assertCurrentRepoSection(page, 'Code')
    await assertPageHeading(page, 'Code')
    await page.getByText('2 files', { exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.html' }).waitFor()
    await page.getByRole('button', { name: 'README.html', exact: true }).waitFor()
    const previewButton = page.getByRole('radio', { name: 'Preview', exact: true })
    const sourceButton = page.getByRole('radio', { name: 'Source', exact: true })
    await previewButton.waitFor()
    await sourceButton.waitFor()

    const preview = page.locator('iframe[title="README.html preview"]')
    await preview.waitFor()
    const sandbox = await preview.getAttribute('sandbox')
    assert.notEqual(sandbox, null)
    const sandboxCapabilities = sandbox.split(/\s+/)
    for (const capability of [
      'allow-forms',
      'allow-popups',
      'allow-popups-to-escape-sandbox',
      'allow-same-origin',
      'allow-scripts',
      'allow-top-navigation',
    ]) {
      assert.equal(sandboxCapabilities.includes(capability), false)
    }

    const previewDocument = page.frameLocator('iframe[title="README.html preview"]')
    await previewDocument
      .getByRole('heading', { level: 1, name: 'Public by design.' })
      .waitFor()
    const contentSecurityPolicy = await previewDocument
      .locator('meta[http-equiv="Content-Security-Policy"]')
      .getAttribute('content')
    assert.match(contentSecurityPolicy, /script-src 'none'/)
    assert.match(contentSecurityPolicy, /connect-src 'none'/)

    const previewElement = await preview.elementHandle()
    assert(previewElement)
    const previewFrame = await previewElement.contentFrame()
    assert(previewFrame)

    let networkRequests = 0
    const networkProbeUrl = 'https://example.com/README-sandbox-check'
    await page.route(networkProbeUrl, async (route) => {
      networkRequests += 1
      await route.fulfill({ status: 204 })
    })
    assert.equal(
      await previewFrame.evaluate(async (url) => {
        try {
          await fetch(url, { mode: 'no-cors' })
          return 'allowed'
        } catch {
          return 'blocked'
        }
      }, networkProbeUrl),
      'blocked',
    )
    assert.equal(networkRequests, 0)
    await previewFrame.evaluate(() => {
      const script = document.createElement('script')
      script.textContent = 'document.documentElement.dataset.scriptRan = "true"'
      document.head.append(script)
    })
    assert.equal(
      await previewFrame.evaluate(() => document.documentElement.dataset.scriptRan),
      undefined,
    )
    assert.equal(
      await previewFrame.evaluate(() => {
        try {
          return parent.document.body !== null
        } catch {
          return false
        }
      }),
      false,
    )

    await previewFrame.evaluate(() => {
      document.documentElement.dataset.persistenceProbe = 'same-document'
    })

    await page.waitForFunction(() => {
      const tab = document.querySelector('[role="tab"][aria-label="README.html"]')
      return tab && Object.keys(tab).some((key) => key.startsWith('__reactProps$'))
    })
    const readmeHistoryLength = await page.evaluate(() => history.length)
    const selectedReadmeRequests = []
    const recordSelectedReadmeRequest = (request) => {
      if (request.url().includes('/_serverFn/')) {
        selectedReadmeRequests.push(request.url())
      }
    }
    page.on('request', recordSelectedReadmeRequest)
    await page.getByRole('button', { name: 'README.html', exact: true }).click()
    await page.getByRole('tab', { name: 'README.html', exact: true }).dblclick()
    page.off('request', recordSelectedReadmeRequest)
    assert.deepEqual(selectedReadmeRequests, [])
    assert.equal(new URL(page.url()).searchParams.has('file'), false)
    assert.equal(await page.evaluate(() => history.length), readmeHistoryLength)
    await preview.waitFor()
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )

    await sourceButton.click()
    await page.locator('pre code').filter({ hasText: '<!doctype html>' }).waitFor()
    assert.equal(await preview.isVisible(), false)
    await previewButton.click()
    await previewDocument
      .getByRole('heading', { level: 1, name: 'Public by design.' })
      .waitFor()
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )

    await page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'Requests', exact: true })
      .click()
    await assertPageHeading(page, 'Requests')
    const codeReturnRequests = []
    const recordCodeReturnRequest = (request) => {
      if (request.url().includes('/_serverFn/')) {
        codeReturnRequests.push(decodeURIComponent(request.url()))
      }
    }
    page.on('request', recordCodeReturnRequest)
    await page.evaluate(() => {
      globalThis.__scopeCodeReturnSawLoading = false
      globalThis.__scopeTrackCodeReturn = true
      const sample = () => {
        if (!globalThis.__scopeTrackCodeReturn) return
        if (document.body.textContent?.includes('Loading repository files')) {
          globalThis.__scopeCodeReturnSawLoading = true
        }
        requestAnimationFrame(sample)
      }
      requestAnimationFrame(sample)
    })
    await page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'Code', exact: true })
      .click()
    await assertPageHeading(page, 'Code')
    await preview.waitFor()
    const sawCodeReturnLoading = await page.evaluate(() => {
      globalThis.__scopeTrackCodeReturn = false
      return globalThis.__scopeCodeReturnSawLoading
    })
    page.off('request', recordCodeReturnRequest)
    assert.equal(sawCodeReturnLoading, false)
    assert.equal(
      codeReturnRequests.some((request) => request.includes('loadRepoContent')),
      false,
    )
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )
    const navigator = page.getByLabel('Repository file navigator')
    await navigator.evaluate((element) => {
      element.dataset.persistenceProbe = 'same-route'
    })
    let appFileRequests = 0
    let releaseFileRequest = () => undefined
    let markFileRequestStarted = () => undefined
    const fileRequestStarted = new Promise((resolve) => {
      markFileRequestStarted = resolve
    })
    const fileRequestReleased = new Promise((resolve) => {
      releaseFileRequest = resolve
    })
    await page.route('**/_serverFn/**', async (route) => {
      if (!decodeURIComponent(route.request().url()).includes('src/app.ts')) {
        await route.continue()
        return
      }
      appFileRequests += 1
      markFileRequestStarted()
      await fileRequestReleased
      await route.continue()
    })
    await page.evaluate(() => {
      globalThis.__scopeTransitionFrames = {
        emptyViewer: false,
        hiddenRepository: false,
      }
      globalThis.__scopeTrackTransitionFrames = true
      const sample = () => {
        if (!globalThis.__scopeTrackTransitionFrames) return
        if (document.body.textContent?.includes(
          'Select a file to inspect its projected contents.',
        )) {
          globalThis.__scopeTransitionFrames.emptyViewer = true
        }
        const navigator = document.querySelector(
          '[aria-label="Repository file navigator"]',
        )
        if (!navigator || navigator.getClientRects().length === 0) {
          globalThis.__scopeTransitionFrames.hiddenRepository = true
        }
        requestAnimationFrame(sample)
      }
      requestAnimationFrame(sample)
    })
    const expandSrc = page.getByRole('button', { name: 'Expand src' })
    await page.waitForFunction(
      (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
      await expandSrc.elementHandle(),
    )
    await expandSrc.click()
    const openFile = page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await within(fileRequestStarted, 10_000, 'file request did not start')
    await page.waitForURL((url) => url.searchParams.get('file') === 'src/app.ts')
    const pendingFileViewer = page.locator(
      '#repository-code-files-panel [aria-busy="true"]',
    )
    await pendingFileViewer.waitFor()
    await assertPassiveSkeleton(page, '#repository-code-files-panel')
    assert.equal(await preview.isVisible(), false)
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    releaseFileRequest()
    await openFile
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(
      await page.locator('#repository-code-files-panel [data-slot="skeleton"]').count(),
      0,
    )
    assert.equal(appFileRequests, 1)
    const transitionFrames = await page.evaluate(() => {
      globalThis.__scopeTrackTransitionFrames = false
      return globalThis.__scopeTransitionFrames
    })
    assert.equal(transitionFrames.emptyViewer, false)
    assert.equal(transitionFrames.hiddenRepository, false)
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    await page.goBack()
    await page.waitForURL((url) => !url.searchParams.has('file'))
    await preview.waitFor()
    assert.equal(
      await navigator.getAttribute('data-persistence-probe'),
      'same-route',
    )
    assert.equal(
      await previewFrame.evaluate(() =>
        document.documentElement.dataset.persistenceProbe
      ),
      'same-document',
    )
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await page.locator('pre code').filter({ hasText: 'export function greet' }).waitFor()
    assert.equal(appFileRequests, 1)
    await page.goBack()
    await page.waitForURL((url) => !url.searchParams.has('file'))
    await preview.waitFor()
    await page.reload()
    await preview.waitFor()
    const failedNavigator = page.getByLabel('Repository file navigator')
    const expandFailedSrc = page.getByRole('button', { name: 'Expand src' })
    await page.waitForFunction(
      (element) => Object.keys(element).some((key) => key.startsWith('__reactProps$')),
      await expandFailedSrc.elementHandle(),
    )
    await failedNavigator.evaluate((element) => {
      element.dataset.persistenceProbe = 'failed-file-local'
    })
    await page.unroute('**/_serverFn/**')
    await page.route('**/_serverFn/**', async (route) => {
      if (!decodeURIComponent(route.request().url()).includes('src/app.ts')) {
        await route.continue()
        return
      }
      await route.fulfill({ body: 'file unavailable', status: 500 })
    })
    await expandFailedSrc.click()
    await page.getByRole('button', { name: 'app.ts', exact: true }).click()
    await page.waitForURL((url) => url.searchParams.get('file') === 'src/app.ts')
    await page.getByRole('button', { name: 'Retry', exact: true }).waitFor()
    assert.equal(
      await failedNavigator.getAttribute('data-persistence-probe'),
      'failed-file-local',
    )
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
    await assertPageHeading(page, 'Runs')
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
    await assertCurrentRepoSection(page, 'History')
    await assertPageHeading(page, 'History')
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
    await assertCurrentRepoSection(page, 'Code')
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
    await assertCurrentRepoSection(page, 'History')
    await page.getByText('Projected public update', { exact: true }).first().waitFor()
    assert.equal(
      await page.evaluate(() => window.__scopeSmokeDocument),
      documentSentinel,
    )
  })
})

test('repository chrome persists across navigation and request revalidation', async () => {
  await withPage(`/${owner}/update-demo`, async (page) => {
    await assertCurrentRepoSection(page, 'Code')
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
    await assertCurrentRepoSection(page, 'Requests')
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
    await assertCurrentRepoSection(page, 'Requests')
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
    await assertCurrentRepoSection(page, 'Requests')

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
    await assertCurrentRepoSection(page, 'History')
    await assertRepositoryChromePreserved(page, { header, navigation })
    await assertCurrentRepoSection(page, 'History')
  })
})

test('requests navigation shows a destination skeleton inside the repository shell', async () => {
  await withPage(repoPath, async (page) => {
    await page.emulateMedia({ reducedMotion: 'reduce' })
    await assertCurrentRepoSection(page, 'Code')
    await assertPageHeading(page, 'Code')
    await page.getByLabel('Repository file navigator').waitFor()
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

    let releaseQueueRequests = () => undefined
    let markQueueRequestStarted = () => undefined
    const queueRequestStarted = new Promise((resolve) => {
      markQueueRequestStarted = resolve
    })
    const queueRequestsReleased = new Promise((resolve) => {
      releaseQueueRequests = resolve
    })
    await page.route('**/_serverFn/**', async (route) => {
      const requestUrl = decodeURIComponent(route.request().url())
      if (!requestUrl.includes('section')) {
        await route.continue()
        return
      }
      markQueueRequestStarted()
      await queueRequestsReleased
      await route.continue()
    })

    const requestsNavigation = page
      .getByRole('navigation', { name: 'Primary' })
      .getByRole('link', { name: 'Requests', exact: true })
      .click()

    try {
      await within(
        queueRequestStarted,
        10_000,
        'request queue navigation did not start',
      )
      const pendingPage = page.locator('#main-content [aria-busy="true"]').first()
      await pendingPage.waitFor()
      await assertCurrentRepoSection(page, 'Requests')
      await assertRepositoryChromePreserved(page, { header, navigation })
      assert.equal(
        await page.getByLabel('Repository file navigator').count(),
        0,
      )
      assert.equal(
        await page.getByRole('heading', { level: 1, name: 'Code' }).count(),
        0,
      )
      await assertPassiveSkeleton(page, '#main-content')
      const reducedMotion = await page
        .locator('#main-content [data-slot="skeleton"]')
        .first()
        .evaluate((element) => {
          const style = getComputedStyle(element)
          return {
            durationSeconds: Number.parseFloat(style.animationDuration),
            iterations: style.animationIterationCount,
          }
        })
      assert.equal(reducedMotion.durationSeconds <= 0.001, true)
      assert.equal(reducedMotion.iterations, '1')
    } finally {
      releaseQueueRequests()
      await requestsNavigation
    }

    await assertPageHeading(page, 'Requests')
    await page.locator('#main-content [data-slot="skeleton"]').first().waitFor({
      state: 'detached',
    })
    assert.equal(
      await page.locator('#main-content [data-slot="skeleton"]').count(),
      0,
    )
  })
})

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await assertCurrentRepoSection(page, 'Requests')
    await assertPageHeading(page, 'Requests')
    await page.getByRole('heading', { level: 2, name: 'Your work' }).waitFor()
    await page.getByRole('heading', { level: 2, name: 'Open' }).waitFor()
    await page.getByRole('heading', { level: 2, name: 'Closed' }).waitFor()
    await page.getByText('No open requests.', { exact: true }).waitFor()
    await page.getByText('No closed requests.', { exact: true }).waitFor()
  })
})

test('seeded request discussion and changes stay reciprocal and ordered', async () => {
  await withPage(`/${owner}/update-demo/requests/req_demo_ready`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Add bounded retry timing' }).waitFor()
    assert.equal(
      await page.getByRole('button', { name: 'Refresh', exact: true }).count(),
      0,
    )
    await page.getByText('Public request', { exact: true }).last().waitFor()
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

    const transitionServerFunctions = []
    const recordServerFunction = (request) => {
      if (request.url().includes('/_serverFn/')) {
        transitionServerFunctions.push(new URL(request.url()).pathname)
      }
    }
    page.on('request', recordServerFunction)
    await changesLink.click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready/changes'))
    await page
      .getByRole('button', { name: /, commit .+, \d+ files?$/ })
      .first()
      .waitFor()
    page.off('request', recordServerFunction)
    const repeatedServerFunctions = transitionServerFunctions.filter(
      (url, index, requests) => requests.indexOf(url) !== index,
    )
    assert.deepEqual(repeatedServerFunctions, [])
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
    })
    await page.getByRole('navigation', { name: 'Request views' })
      .getByRole('link', { name: 'Discussion' })
      .click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready'))
    await page.locator('.request-discussion-thread').first().waitFor()
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
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
      await page.getByText(/^\d+ open$/).waitFor()
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
      // Open and Closed now share the same no-match copy, so scope by section.
      await page
        .getByRole('region', { name: 'Open' })
        .getByText('Nothing matches “missing request title”.', { exact: true })
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

async function within(promise, timeoutMs, message) {
  let timeout
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timeout)
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

async function assertPageHeading(page, title) {
  await page.getByRole('heading', { level: 1, name: title }).waitFor({
    state: 'attached',
  })
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

async function assertPassiveSkeleton(page, selector) {
  const skeletons = page.locator(`${selector} [data-slot="skeleton"]:visible`)
  await skeletons.first().waitFor()
  assert.equal((await skeletons.count()) > 0, true)
  assert.deepEqual(
    await skeletons.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute('aria-hidden')),
    ),
    Array.from({ length: await skeletons.count() }, () => 'true'),
  )
  assert.equal(
    await page.locator(`${selector} .animate-spin:visible`).count(),
    0,
  )
  assert.deepEqual(
    await page.locator(selector).evaluate((root) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
      const visibleLoadingText = []
      let node = walker.nextNode()
      while (node) {
        const text = node.textContent?.trim() ?? ''
        const parent = node.parentElement
        if (
          /^(Loading|Connecting)\b/i.test(text) &&
          parent &&
          !parent.closest('.sr-only') &&
          parent.getClientRects().length > 0
        ) {
          visibleLoadingText.push(text)
        }
        node = walker.nextNode()
      }
      return visibleLoadingText
    }),
    [],
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
