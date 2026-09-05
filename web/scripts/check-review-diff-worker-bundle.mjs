import assert from 'node:assert/strict'
import { readdir, readFile, stat } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { join } from 'node:path'
import { Worker } from 'node:worker_threads'

const MAX_BUNDLE_BYTES = 11 * 1024 * 1024
const MAX_BUNDLE_FILES = 350
const workerDirectory = fileURLToPath(new URL(
  '../.output/server/_workers',
  import.meta.url,
))
const workerEntry = join(
  workerDirectory,
  'review-file-diff-render-worker.mjs',
)
const serverSsrDirectory = fileURLToPath(new URL(
  '../.output/server/_ssr',
  import.meta.url,
))

const workerFiles = await filesBelow(workerDirectory)
const bundleBytes = (await Promise.all(
  workerFiles.map(async (path) => (await stat(path)).size),
)).reduce((total, size) => total + size, 0)

assert.ok(workerFiles.includes(workerEntry), 'review diff worker entry was not emitted')
assert.ok(
  bundleBytes <= MAX_BUNDLE_BYTES,
  `review diff worker bundle is ${bundleBytes} bytes; budget is ${MAX_BUNDLE_BYTES}`,
)
assert.ok(
  workerFiles.length <= MAX_BUNDLE_FILES,
  `review diff worker emitted ${workerFiles.length} files; budget is ${MAX_BUNDLE_FILES}`,
)

const ssrSources = await Promise.all(
  (await filesBelow(serverSsrDirectory))
    .filter((path) => path.endsWith('.mjs'))
    .map((path) => readFile(path, 'utf8')),
)
const prerenderSource = ssrSources.find((source) =>
  source.includes('scope.review-file-diff-renderer-v1')
)
assert.ok(prerenderSource, 'review diff server renderer was not emitted')
assert.match(
  prerenderSource,
  /\.\.\/_workers\/review-file-diff-render-worker\.mjs/,
  'review diff server renderer does not reference the emitted worker',
)
assert.doesNotMatch(
  prerenderSource,
  /review-file-diff-render-worker\.ts/,
  'review diff server renderer still references TypeScript source',
)

const workerBudget = {
  maxHighlightLanguages: 16,
  maxHunks: 64,
  maxOutputBytes: 256 * 1024,
  maxRenderedLines: 800,
}
assert.deepEqual(
  await runWorker({
    budget: workerBudget,
    newText: 'same\n',
    oldText: 'same\n',
    path: '/fixture.ts',
  }),
  { kind: 'empty' },
)
const rendered = await runWorker({
  budget: workerBudget,
  newText: 'export const value = 2\n',
  oldText: 'export const value = 1\n',
  path: '/fixture.ts',
})
assert.equal(rendered.kind, 'html')
assert.ok(Buffer.byteLength(rendered.html) <= workerBudget.maxOutputBytes)

console.log(
  `review diff worker: ${workerFiles.length} files, ${bundleBytes} bytes, executable`,
)

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  return (await Promise.all(entries.map(async (entry) => {
    const path = join(directory, entry.name)
    return entry.isDirectory() ? filesBelow(path) : [path]
  }))).flat()
}

function runWorker(workerData) {
  const worker = new Worker(pathToFileURL(workerEntry))
  return new Promise((resolve, reject) => {
    let settled = false
    const deadline = setTimeout(() => {
      void finish(new Error('built review diff worker missed its smoke deadline'))
    }, 10_000)
    worker.once('message', (message) => void finish(null, message))
    worker.once('error', (error) => void finish(error))
    worker.postMessage(workerData)
    worker.once('exit', (code) => {
      if (code !== 0) void finish(new Error(`built review diff worker exited ${code}`))
    })

    async function finish(error, result) {
      if (settled) return
      settled = true
      clearTimeout(deadline)
      try {
        await worker.terminate()
      } catch {
        // Preserve the smoke-test outcome if Node reports a termination failure.
      }
      if (error) reject(error)
      else resolve(result)
    }
  })
}
