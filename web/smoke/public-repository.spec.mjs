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
    await page.getByRole('heading', { level: 1, name: repoId }).waitFor()
    await assertCurrentRepoSection(page, 'Code')
    await page.getByRole('heading', { level: 2, name: 'Source' }).waitFor()
    await page.getByText('2 files', { exact: true }).waitFor()
    await page.getByText('README.md', { exact: true }).waitFor()
    assert.equal(await page.getByText('internal', { exact: true }).count(), 0)
    assert.equal(await page.getByText('plan.md', { exact: true }).count(), 0)
  })
})

test('public repository history renders its seeded commit', async () => {
  await withPage(`${repoPath}/history`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'History' }).waitFor()
    await assertCurrentRepoSection(page, 'History')
    await page.getByRole('heading', { level: 2, name: 'Commits' }).waitFor()
    await page.getByText('dev-public-1', { exact: true }).first().waitFor()
    await page.getByText('Projected public update', { exact: true }).first().waitFor()
  })
})

test('public repository requests route is anonymously readable', async () => {
  await withPage(`${repoPath}/requests`, async (page) => {
    await page.getByRole('heading', { level: 1, name: 'Requests' }).waitFor()
    await assertCurrentRepoSection(page, 'Requests')
    await page.getByText('No requests yet', { exact: true }).waitFor()
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
    .getByRole('navigation', { name: 'Repository' })
    .getByRole('link', { name: section, exact: true })
  await link.waitFor()
  assert.equal(await link.getAttribute('aria-current'), 'page')
}
