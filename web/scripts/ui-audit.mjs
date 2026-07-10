import { chromium } from 'playwright'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'

const args = parseArgs(process.argv.slice(2))
const outputDir = process.env.UI_AUDIT_DIR
if (!outputDir) {
  throw new Error('UI_AUDIT_DIR is required')
}

const baseUrl = (args['base-url'] ?? 'http://localhost:3000').replace(/\/$/, '')
const auditRepo = process.env.UI_AUDIT_REPO ?? 'dev/public-demo'
const repoRoute = `/repos/${auditRepo.replace(/^\/+|\/+$/g, '')}`
const routes = (args.routes ?? `${repoRoute},${repoRoute}/requests,${repoRoute}/history`)
  .split(',')
  .map((route) => route.trim())
  .filter(Boolean)
const viewports = [
  { name: 'desktop', width: 1440, height: 1000 },
  { name: 'mobile', width: 390, height: 844 },
]
const screenshotDir = path.join(outputDir, 'screenshots')
await mkdir(screenshotDir, { recursive: true })

const browser = await chromium.launch({ headless: true })
const results = []
try {
  for (const route of routes) {
    for (const viewport of viewports) {
      const page = await browser.newPage({ viewport })
      const consoleErrors = []
      const pageErrors = []
      page.on('console', (message) => {
        if (message.type() === 'error') consoleErrors.push(message.text())
      })
      page.on('pageerror', (error) => pageErrors.push(error.message))

      const url = new URL(route, `${baseUrl}/`).toString()
      let navigationError = null
      let responseStatus = null
      try {
        const response = await page.goto(url, {
          waitUntil: 'domcontentloaded',
          timeout: 30_000,
        })
        responseStatus = response?.status() ?? null
        await page.waitForTimeout(500)
      } catch (error) {
        navigationError = error instanceof Error ? error.message : String(error)
      }

      const overflow = navigationError
        ? null
        : await page.evaluate(() => {
            const documentWidth = document.documentElement.scrollWidth
            const viewportWidth = window.innerWidth
            const offenders = [...document.querySelectorAll('body *')]
              .map((element) => {
                const rect = element.getBoundingClientRect()
                return { element, left: rect.left, right: rect.right, width: rect.width }
              })
              .filter(({ left, right, width }) => width > 0 && (left < -1 || right > viewportWidth + 1))
              .slice(0, 12)
              .map(({ element, left, right, width }) => ({
                className: element.className?.toString().slice(0, 160) ?? '',
                left: Math.round(left),
                right: Math.round(right),
                tag: element.tagName.toLowerCase(),
                width: Math.round(width),
              }))
            return {
              documentWidth,
              offenders,
              viewportWidth,
              overflowed: documentWidth > viewportWidth + 1,
            }
          })
      const fileName = `${safeName(route)}-${viewport.name}.png`
      await page.screenshot({
        fullPage: true,
        path: path.join(screenshotDir, fileName),
      })
      results.push({
        consoleErrors,
        navigationError,
        overflow,
        pageErrors,
        responseStatus,
        route,
        screenshot: `screenshots/${fileName}`,
        viewport,
      })
      await page.close()
    }
  }
} finally {
  await browser.close()
}

const failures = results.filter(
  (result) =>
    result.navigationError ||
    (result.responseStatus !== null && result.responseStatus >= 400) ||
    result.consoleErrors.length > 0 ||
    result.pageErrors.length > 0 ||
    result.overflow?.overflowed,
)
const report = {
  baseUrl,
  generatedAt: new Date().toISOString(),
  results,
  routes,
  summary: {
    failed: failures.length,
    passed: results.length - failures.length,
    total: results.length,
  },
}
await writeFile(
  path.join(outputDir, 'report.json'),
  `${JSON.stringify(report, null, 2)}\n`,
)

if (failures.length > 0) {
  process.exitCode = 1
}

function parseArgs(values) {
  const parsed = {}
  for (let index = 0; index < values.length; index += 1) {
    const key = values[index]
    if (!key?.startsWith('--')) continue
    parsed[key.slice(2)] = values[index + 1]
    index += 1
  }
  return parsed
}

function safeName(route) {
  return route.replace(/^\//, '').replace(/[^a-z0-9]+/gi, '-').replace(/-$/, '') || 'home'
}
