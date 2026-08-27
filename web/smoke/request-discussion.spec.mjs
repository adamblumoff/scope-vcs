import assert from 'node:assert/strict'
import { test } from 'node:test'
import { chromium } from 'playwright'
import {
  assertFileSelectionSkipsRevisionReload,
  assertRequestCrossLinksStayInDocument,
  assertRequestShellPreserved,
  assertUpdateSelectionUsesInitialPayload,
  waitForClientHydration,
} from './request-changes-smoke.mjs'

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
    assert.equal(await page.getByRole('link', { name: 'Link to discussion' }).count(), 0)
    assert.equal(await page.getByRole('link', { name: 'Link to reply' }).count(), 0)
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

    const resolvedThread = page.locator('#discussion-discussion_demo_resolved_docs')
    await resolvedThread.getByRole('button', { name: 'Show 1 reply' }).waitFor()
    await resolvedThread.getByText('The helper accepts milliseconds', {
      exact: false,
    }).waitFor()

    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).waitFor()
    const maintainerReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_maintainer',
    )
    await maintainerReply.getByText('Two seconds is intentional', { exact: false }).waitFor()
    const contributorReply = page.locator(
      '#reply-discussion_reply_demo_retry_cap_quote',
    )
    await contributorReply.getByText('Agreed. Quoting the maintainer', { exact: false }).waitFor()
    const nestedReply = page.locator('#reply-discussion_reply_demo_retry_cap_nested')
    await nestedReply.getByText('Exactly. Keeping that decision', { exact: false }).waitFor()
    assert.deepEqual(
      await retryThread.locator('[id^="reply-"]').evaluateAll((elements) =>
        elements.map(({ id }) => id),
      ),
      [
        'reply-discussion_reply_demo_retry_cap_maintainer',
        'reply-discussion_reply_demo_retry_cap_quote',
        'reply-discussion_reply_demo_retry_cap_nested',
      ],
    )
    await contributorReply
      .locator(
        'a[href="#discussion=discussion_demo_retry_cap&reply=discussion_reply_demo_retry_cap_maintainer"]',
      )
      .getByText('Two seconds is intentional', { exact: false })
      .waitFor()
    await nestedReply
      .locator(
        'a[href="#discussion=discussion_demo_retry_cap&reply=discussion_reply_demo_retry_cap_quote"]',
      )
      .getByText('Agreed. Quoting the maintainer', { exact: false })
      .waitFor()

    const hideRetryReplies = retryThread.getByRole('button', {
      name: 'Hide 3 replies',
    })
    const retryReplies = retryThread.locator(
      '#discussion-discussion_demo_retry_cap-replies',
    )
    await waitForClientHydration(page, hideRetryReplies)
    const disclosureTop = await hideRetryReplies.evaluate(
      (element) => element.getBoundingClientRect().top,
    )
    const scrollPosition = await page.evaluate(() => ({
      main: document.getElementById('main-content')?.scrollTop ?? null,
      window: window.scrollY,
    }))
    await hideRetryReplies.click()
    await assertReplyRegion(page, retryReplies, false)
    await retryThread
      .getByText('Should the retry cap remain', { exact: false })
      .waitFor()
    assert.equal(
      Math.round(
        await retryThread
          .getByRole('button', { name: 'Show 3 replies' })
          .evaluate((element) => element.getBoundingClientRect().top),
      ),
      Math.round(disclosureTop),
    )
    assert.deepEqual(
      await page.evaluate(() => ({
        main: document.getElementById('main-content')?.scrollTop ?? null,
        window: window.scrollY,
      })),
      scrollPosition,
    )
    await retryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, true)
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).click()
    await retryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, true)
    await retryThread.getByRole('button', { name: 'Hide 3 replies' }).click()
    await assertReplyRegion(page, retryReplies, false)

    const jitterThread = page.locator('#discussion-discussion_demo_jitter')
    const hideJitterReplies = jitterThread.getByRole('button', {
      name: 'Hide 1 reply',
    })
    const mainContent = page.locator('#main-content')
    await page.evaluate(() => {
      window.location.hash =
        '#discussion=discussion_demo_jitter&reply=discussion_reply_demo_jitter'
    })
    await page.locator('#reply-discussion_reply_demo_jitter').waitFor()
    await hideJitterReplies.evaluate((element) => {
      element.scrollIntoView({ block: 'center' })
    })
    const deepScrollPosition = await mainContent.evaluate(
      (element) => element.scrollTop,
    )
    assert(deepScrollPosition > 0)
    await hideJitterReplies.click()
    await assertReplyRegion(
      page,
      jitterThread.locator('#discussion-discussion_demo_jitter-replies'),
      false,
    )
    assert.equal(
      await mainContent.evaluate((element) => element.scrollTop),
      deepScrollPosition,
    )

    const {
      heading: requestHeading,
      navigation: requestNavigation,
    } = await assertRequestCrossLinksStayInDocument(page)

    const requestViews = page.getByRole('navigation', { name: 'Request views' })
    const changesLink = requestViews.getByRole('link', { name: 'Changes' })
    await waitForClientHydration(page, changesLink)
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
    await assertFileSelectionSkipsRevisionReload(page, 'retry.ts', '/src/retry.ts')
    await assertUpdateSelectionUsesInitialPayload(page)
    await page.getByRole('navigation', { name: 'Request views' })
      .getByRole('link', { name: 'Discussion' })
      .click()
    await page.waitForURL((url) => url.pathname.endsWith('/requests/req_demo_ready'))
    await page.locator('.request-discussion-thread').first().waitFor()
    const restoredRetryThread = page.locator('#discussion-discussion_demo_retry_cap')
    await restoredRetryThread
      .getByRole('button', { name: 'Show 3 replies' })
      .waitFor()
    const restoredRetryReplies = restoredRetryThread.locator(
      '#discussion-discussion_demo_retry_cap-replies',
    )
    await assertReplyRegion(page, restoredRetryReplies, false)
    await restoredRetryThread.getByRole('button', { name: 'Show 3 replies' }).click()
    await assertReplyRegion(page, restoredRetryReplies, true)
    await assertRequestShellPreserved(page, {
      heading: requestHeading,
      navigation: requestNavigation,
    })
  })
})

async function assertReplyRegion(page, region, expanded) {
  const id = await region.getAttribute('id')
  assert(id, 'reply region must have an id')
  await page.waitForFunction(
    ({ expectedExpanded, regionId }) => {
      const element = document.getElementById(regionId)
      if (!element) return false
      const height = element.getBoundingClientRect().height
      return expectedExpanded ? height > 0 : height < 1
    },
    { expectedExpanded: expanded, regionId: id },
  )
  assert.equal(await region.getAttribute('aria-hidden'), String(!expanded))
  assert.equal(await region.getAttribute('inert'), expanded ? null : '')
}

async function withPage(path, assertion) {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage()
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
