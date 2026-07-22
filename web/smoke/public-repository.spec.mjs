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

const repoPath = `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`

test('public repository exposes only its projected source', async () => {
  await withPage(repoPath, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Repository' }).waitFor()
    await assertCurrentRepoSection(page, 'Code')
    await page.getByText('2 files', { exact: true }).waitFor()
    await page.getByRole('tab', { name: 'README.md' }).waitFor()
    await page.getByRole('button', { name: 'README.md', exact: true }).waitFor()
    assert.equal(await page.getByText('internal', { exact: true }).count(), 0)
    assert.equal(await page.getByText('plan.md', { exact: true }).count(), 0)
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

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()
    await assertCurrentRepoSection(page, 'Requests')
    await page.getByText('No requests yet', { exact: true }).waitFor()
  })
})

test('seeded request timeline keeps its order and exposes nested reply branches', async () => {
  await withPage(`/repos/${owner}/update-demo/requests/req_demo_submitted`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Add bounded retry timing' }).waitFor()
    const threads = page.locator('.request-discussion-thread')
    await threads.first().waitFor()
    assert.deepEqual(
      await threads.evaluateAll((elements) => elements.map(({ id }) => id)),
      [
        'discussion-thread_event_req_demo_submitted_submitted',
        'discussion-thread_event_req_demo_submitted_revision_1',
        'discussion-thread_event_req_demo_submitted_revision_2',
        'discussion-thread_event_req_demo_submitted_revision_3',
        'discussion-thread_event_req_demo_submitted_revision_4',
        'discussion-discussion_demo_retry_cap',
        'discussion-discussion_demo_jitter',
        'discussion-discussion_demo_resolved_docs',
      ],
    )

    const retryThread = page.locator('#discussion-discussion_demo_retry_cap')
    await retryThread.getByRole('button', { name: '3 replies' }).click()
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
  })
})

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

async function assertCurrentRepoSection(page, section) {
  const link = page
    .getByRole('navigation', { name: 'Primary' })
    .getByRole('link', { name: section, exact: true })
  await link.waitFor()
  assert.equal(await link.getAttribute('aria-current'), 'page')
}
