import assert from 'node:assert/strict'
import test from 'node:test'
import { Worker } from 'node:worker_threads'
import { REVIEW_FILE_DIFF_RENDER_BUDGET } from './review-file-diff-render-contract'
import { ReviewDiffTransientError } from './review-file-diff-renderer'
import { createReviewFileDiffWorkerPool } from './review-file-diff-worker-pool'

const workerCode = `
  const { parentPort, threadId } = require('node:worker_threads');
  let count = 0;
  parentPort.on('message', (input) => {
    if (input.newText === 'spin') while (true) {}
    if (input.newText === 'crash') throw new Error('fixture crash');
    if (input.newText === 'exit') process.exit(0);
    setTimeout(() => parentPort.postMessage({
      kind: 'html', html: threadId + ':' + (++count)
    }), 20);
  });
`

function input(newText = 'text') {
  return { budget: REVIEW_FILE_DIFF_RENDER_BUDGET, oldText: '', newText, path: 'fixture.ts' }
}

test('reuses idle workers and rejects overload without a queue', async (t) => {
  const workers: Worker[] = []
  const render = createReviewFileDiffWorkerPool(() => {
    const worker = new Worker(workerCode, { eval: true })
    workers.push(worker)
    return worker
  }, 2)
  t.after(() => Promise.all(workers.map((worker) => worker.terminate())))
  const first = render(input(), 1_000)
  const second = render(input(), 1_000)
  await assert.rejects(render(input(), 1_000), (error: unknown) =>
    error instanceof ReviewDiffTransientError && error.failure === 'busy')
  const [a] = await Promise.all([first, second])
  const reused = await render(input(), 1_000)
  assert.equal(workers.length, 2)
  assert.equal(a.kind, 'html')
  assert.equal(reused.kind, 'html')
  if (a.kind === 'html' && reused.kind === 'html') {
    assert.equal(a.html.split(':')[0], reused.html.split(':')[0])
    assert.ok(reused.html.endsWith(':2'))
  }
})

test('terminates timed out, canceled and failed tasks and replaces their workers', async (t) => {
  const workers: Worker[] = []
  const render = createReviewFileDiffWorkerPool(() => {
    const worker = new Worker(workerCode, { eval: true })
    workers.push(worker)
    return worker
  }, 1)
  t.after(() => Promise.all(workers.map((worker) => worker.terminate())))
  await render(input(), 1_000)
  await assert.rejects(render(input('spin'), 25), (error: unknown) =>
    error instanceof ReviewDiffTransientError && error.failure === 'deadline')
  assert.equal(workers[0]?.threadId, -1)
  await render(input(), 1_000)
  const controller = new AbortController()
  const pending = render(input('spin'), 1_000, controller.signal)
  controller.abort()
  await assert.rejects(pending, { name: 'AbortError' })
  assert.equal(workers[1]?.threadId, -1)
  for (const fixture of ['crash', 'exit']) {
    await assert.rejects(render(input(fixture), 1_000))
    assert.equal(workers.at(-1)?.threadId, -1)
  }
  assert.equal((await render(input(), 1_000)).kind, 'html')
  assert.equal(workers.length, 5)
})
