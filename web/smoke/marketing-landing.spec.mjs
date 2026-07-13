import assert from 'node:assert/strict'
import { after, before, test } from 'node:test'
import { chromium } from 'playwright'

let browser

before(async () => {
  browser = await chromium.launch({ headless: true })
})

after(async () => {
  await browser?.close()
})

const baseUrl = (
  process.env.SCOPE_WEB_BASE_URL ??
  process.env.PLAYWRIGHT_BASE_URL ??
  'http://localhost:3000'
).replace(/\/$/, '')

test('signed-out root presents the Scope marketing page', async () => {
  await withPage({ width: 1440, height: 900 }, async (page, response) => {
    assert.equal(response.url(), `${baseUrl}/`)
    await page.getByRole('heading', { level: 1, name: 'Open source. On your terms.' }).waitFor()

    assert.equal(
      await page.getByRole('heading', { level: 1 }).count(),
      1,
      'marketing page should have exactly one level-one heading',
    )

    await assertAuthLink(page, 'Sign in', '/sign-in')
    await assertAuthLink(page, 'Create account', '/sign-up')
    await assertAuthLink(page, 'Create your Scope', '/sign-up')

    const { connectors, privateView, projection, publicView } = projectionLocators(page)
    await publicView.waitFor()
    await privateView.getByText('internal/', { exact: true }).waitFor()
    await privateView.getByText('.env', { exact: true }).waitFor()
    await projection.locator('.marketing-connection-desktop').first().waitFor()

    assert.equal(await projection.locator('.marketing-aperture').count(), 0)
    assert.equal(await projection.locator('button').count(), 0)
    assert.equal(await connectors.count(), 6)
    assert.equal((await visibleConnectorClasses(connectors)).length, 2)
    assert.equal(await publicView.getByText('internal/', { exact: true }).count(), 0)
    assert.equal(await publicView.getByText('.env', { exact: true }).count(), 0)
    assert.equal(await publicView.getByRole('img', { name: 'Public visibility' }).count(), 5)
    assert.equal(await privateView.getByRole('img', { name: 'Mixed visibility' }).count(), 1)
    assert.equal(await privateView.getByRole('img', { name: 'Private visibility' }).count(), 2)
    assert.deepEqual(await projectionPaths(publicView), [
      '/src',
      '/src/cli',
      'src/cli/index.ts',
      '/src/shared',
      'README.md',
    ])
    assert.deepEqual(await projectionPaths(privateView), [
      '/src',
      '/src/cli',
      'src/cli/index.ts',
      '/src/internal',
      '/src/shared',
      '.env',
      'README.md',
    ])
    assert.equal(await publicView.getByText('README.md', { exact: true }).count(), 1)
    assert.equal(await privateView.getByText('README.md', { exact: true }).count(), 1)
    await assertProjectionTreeAlignment(publicView)
    await assertProjectionTreeAlignment(privateView)
    const secondaryCta = page.getByRole('link', { name: 'See how it works' })
    assert.equal(await secondaryCta.getAttribute('href'), '#repository-source')
    await secondaryCta.focus()
    assert.equal(await secondaryCta.evaluate((node) => node === document.activeElement), true)
  })
})

test('hovering projection rows previews linked permissions', async () => {
  await withPage({ width: 1440, height: 900 }, async (page) => {
    await page.waitForLoadState('networkidle')
    await hideClerkDevelopmentPrompt(page)
    const { privateView, projection, publicView, repository } = projectionLocators(page)
    const publicCli = publicView.locator('[data-path="/src/cli"]')
    const privateCli = privateView.locator('[data-path="/src/cli"]')
    const privateInternal = privateView.locator('[data-path="/src/internal"]')
    const publicConnector = projection.locator(
      '.marketing-connection-public.marketing-connection-desktop',
    )

    await publicCli.hover()
    await waitForAttribute(publicCli, 'data-highlighted', 'true')
    await waitForAttribute(privateCli, 'data-highlighted', 'true')
    await waitForAttribute(repository, 'data-source-context', 'src/cli/')

    await privateInternal.hover()
    await waitForAttribute(privateInternal, 'data-highlighted', 'true')
    await waitForAttribute(projection, 'data-private-only', 'true')
    await waitForAttribute(repository, 'data-source-context', 'src/internal/')
    await waitForOpacity(publicView, 'below', 0.3)
    await waitForOpacity(publicConnector, 'below', 0.3)
    assert((await elementOpacity(privateView)) > 0.95)

    await page.mouse.move(0, 0)
    await waitForAttribute(projection, 'data-private-only', null)
    await waitForAttribute(repository, 'data-source-context', 'repository root')
    await waitForOpacity(publicView, 'above', 0.95)
    await waitForOpacity(publicConnector, 'above', 0.95)
  })
})

test('projection remains separated and reachable across responsive layouts', async () => {
  const viewports = [
    { width: 320, height: 568 },
    { width: 360, height: 844 },
    { width: 390, height: 844 },
    { width: 1024, height: 600 },
    { width: 1199, height: 700 },
    { width: 1200, height: 600 },
    { width: 1440, height: 900 },
  ]

  for (const viewport of viewports) {
    await withPage(viewport, async (page) => {
      await page.getByRole('heading', { level: 1, name: 'Open source. On your terms.' }).waitFor()
      await assertProjectionLayout(page, viewport)
    })
  }
})

test('mobile CTA reveals the projection without horizontal clipping', async () => {
  await withPage({ width: 390, height: 844 }, async (page) => {
    const { privateView, publicView, repository } = projectionLocators(page)
    await page.getByRole('link', { name: 'See how it works' }).click()
    await page.waitForFunction(() => window.location.hash === '#repository-source')

    const repositoryBox = await requiredBoundingBox(repository)
    assert(repositoryBox.y >= 0 && repositoryBox.y + repositoryBox.height <= 844)

    const dimensions = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }))
    assert.equal(dimensions.scrollWidth, dimensions.clientWidth)
    assertInsideViewport(repositoryBox, 390)
    assertInsideViewport(await requiredBoundingBox(publicView), 390)
    assertInsideViewport(await requiredBoundingBox(privateView), 390)
  })
})

test('reduced motion disables marketing entrances', async () => {
  await withPage(
    { width: 1440, height: 900 },
    async (page) => {
      const { projection, publicView } = projectionLocators(page)
      const connector = projection.locator('.marketing-connection-desktop').first()
      await connector.waitFor()
      assert.equal(
        await projection.evaluate((node) => getComputedStyle(node).animationName),
        'none',
      )
      assert.equal(
        await publicView.evaluate((node) => getComputedStyle(node).animationName),
        'none',
      )
      assert.equal(
        await connector.evaluate((path) => getComputedStyle(path).animationName),
        'none',
      )
    },
    { reducedMotion: 'reduce' },
  )
})

async function hideClerkDevelopmentPrompt(page) {
  const prompt = page.getByRole('button', { name: 'Keyless prompt' })
  if (await prompt.count()) {
    await prompt.evaluate((node) => {
      node.parentElement.style.display = 'none'
    })
  }
}

function projectionLocators(page) {
  const projection = page.getByRole('region', {
    name: 'One repository projected into public and private views',
  })
  return {
    connectors: projection.locator('path.marketing-connection'),
    privateView: projection
      .getByRole('heading', { name: 'Private view', exact: true })
      .locator('xpath=ancestor::article'),
    projection,
    publicView: projection
      .getByRole('heading', { name: 'Public view', exact: true })
      .locator('xpath=ancestor::article'),
    repository: projection.locator('[data-projection-node="repository"]'),
  }
}

async function assertProjectionLayout(page, viewport) {
  const { connectors, privateView, publicView, repository } = projectionLocators(page)
  const arena = await requiredBoundingBox(page.locator('.marketing-arena'))
  const copy = await requiredBoundingBox(page.locator('.marketing-copy'))
  const repositoryBox = await requiredBoundingBox(repository)
  const publicBox = await requiredBoundingBox(publicView)
  const privateBox = await requiredBoundingBox(privateView)
  const expectedConnectorVariant = connectorVariantForWidth(viewport.width)
  const connectorClasses = await visibleConnectorClasses(connectors)
  await assertProjectionTreeAlignment(publicView)
  await assertProjectionTreeAlignment(privateView)
  assert.equal(connectorClasses.length, 2)
  assert(connectorClasses.every((className) => className?.includes(expectedConnectorVariant)))

  for (const [name, box] of [
    ['Repository', repositoryBox],
    ['Public view', publicBox],
    ['Private view', privateBox],
  ]) {
    assert(rectangleInside(box, arena), `${name} escaped its arena at ${viewport.width}×${viewport.height}`)
    assert(rectanglesAreSeparated(box, copy, 10), `${name} overlapped hero copy at ${viewport.width}×${viewport.height}`)
    assertInsideViewport(box, viewport.width)
  }

  assert(rectanglesAreSeparated(repositoryBox, publicBox, 10), 'Repository overlapped Public view')
  assert(rectanglesAreSeparated(repositoryBox, privateBox, 10), 'Repository overlapped Private view')
  assert(rectanglesAreSeparated(publicBox, privateBox, 10), 'Public and Private views overlapped')
}

function connectorVariantForWidth(width) {
  if (width < 360) return 'marketing-connection-compact'
  if (width < 1200) return 'marketing-connection-stacked'
  return 'marketing-connection-desktop'
}

async function assertProjectionTreeAlignment(view) {
  const src = view.locator('[data-path="/src"]')
  const cli = view.locator('[data-path="/src/cli"]')
  const indexFile = view.locator('[data-path="src/cli/index.ts"]')
  const rootChevronX = await requiredX(src.locator('.lucide-chevron-down'))
  const rootFolderX = await requiredX(src.locator('.lucide-folder-open'))

  for (const path of ['README.md', '.env']) {
    const row = view.locator(`[data-path="${path}"]`)
    if (await row.count() === 0) continue

    assertClose(
      await requiredX(row.locator('.lucide-file')),
      rootChevronX,
      `${path} icon and root chevron`,
    )
    assertClose(
      await requiredX(row.getByText(path, { exact: true })),
      rootFolderX,
      `${path} label and root folder icon`,
    )
  }

  const cliChevronX = await requiredX(cli.locator('.lucide-chevron-down'))
  const cliFolderX = await requiredX(cli.locator('.lucide-folder-open'))
  const indexIconX = await requiredX(indexFile.locator('.lucide-file'))
  const indexLabelX = await requiredX(indexFile.getByText('index.ts', { exact: true }))
  assertClose(indexIconX - cliChevronX, 16, 'file icon depth increment')
  assertClose(indexLabelX - cliFolderX, 16, 'file label depth increment')
}

async function requiredX(locator) {
  return (await requiredBoundingBox(locator)).x
}

function assertClose(actual, expected, label) {
  const difference = Math.abs(actual - expected)
  assert(difference <= 0.5, `${label} differed by ${difference}px`)
}

async function projectionPaths(view) {
  return view.locator('[data-path]').evaluateAll((rows) =>
    rows.map((row) => row.getAttribute('data-path'))
  )
}

async function visibleConnectorClasses(locator) {
  return locator.evaluateAll((paths) =>
    paths
      .filter((path) => getComputedStyle(path).display !== 'none')
      .map((path) => path.getAttribute('class'))
  )
}

async function waitForAttribute(locator, name, expectedValue) {
  await locator.evaluate(
    (node, expectation) => new Promise((resolve, reject) => {
      const deadline = performance.now() + 1_000
      const check = () => {
        const value = node.getAttribute(expectation.name)
        if (value === expectation.expectedValue) {
          resolve(undefined)
          return
        }
        if (performance.now() >= deadline) {
          reject(new Error(`${expectation.name} remained ${value}`))
          return
        }
        requestAnimationFrame(check)
      }
      check()
    }),
    { expectedValue, name },
  )
}

async function elementOpacity(locator) {
  return Number(await locator.evaluate((node) => getComputedStyle(node).opacity))
}

async function waitForOpacity(locator, direction, threshold) {
  await locator.evaluate(
    (node, expectation) => new Promise((resolve, reject) => {
      const deadline = performance.now() + 1_000
      const check = () => {
        const opacity = Number(getComputedStyle(node).opacity)
        const matches = expectation.direction === 'above'
          ? opacity > expectation.threshold
          : opacity < expectation.threshold
        if (matches) {
          resolve(undefined)
          return
        }
        if (performance.now() >= deadline) {
          reject(new Error(`opacity remained ${opacity}`))
          return
        }
        requestAnimationFrame(check)
      }
      check()
    }),
    { direction, threshold },
  )
}

async function requiredBoundingBox(locator) {
  const box = await locator.boundingBox()
  assert(box, 'expected element to have a bounding box')
  return box
}

function rectangleInside(inner, outer) {
  return (
    inner.x >= outer.x - 1 &&
    inner.y >= outer.y - 1 &&
    inner.x + inner.width <= outer.x + outer.width + 1 &&
    inner.y + inner.height <= outer.y + outer.height + 1
  )
}

function assertInsideViewport(box, viewportWidth) {
  assert(box.x >= -1, 'projection content was clipped on the left')
  assert(box.x + box.width <= viewportWidth + 1, 'projection content was clipped on the right')
}

function rectanglesAreSeparated(first, second, gap) {
  return (
    first.x + first.width + gap <= second.x ||
    second.x + second.width + gap <= first.x ||
    first.y + first.height + gap <= second.y ||
    second.y + second.height + gap <= first.y
  )
}

async function assertAuthLink(page, name, expectedPath) {
  const href = await page.getByRole('link', { name, exact: true }).getAttribute('href')
  assert(href, `${name} should have an href`)
  const url = new URL(href, `${baseUrl}/`)
  assert.equal(url.pathname, expectedPath)
  assert.equal(url.searchParams.get('redirect_url'), '/')
}

async function withPage(viewport, assertion, contextOptions = {}) {
  assert(browser, 'browser should be initialized before tests run')
  const context = await browser.newContext({ ...contextOptions, viewport })
  const page = await context.newPage()
  const consoleErrors = []
  const pageErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  page.on('pageerror', (error) => pageErrors.push(error.message))

  try {
    const response = await page.goto(`${baseUrl}/`, {
      timeout: 30_000,
      waitUntil: 'domcontentloaded',
    })
    assert(response, 'navigation to / did not produce a response')
    assert(response.status() < 400, `navigation to / returned ${response.status()}`)
    await page.evaluate(() => document.fonts.ready)
    await assertion(page, response)
    assert.deepEqual(pageErrors, [])
    assert.deepEqual(consoleErrors, [])
  } finally {
    await context.close()
  }
}
