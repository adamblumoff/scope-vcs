import assert from 'node:assert/strict'
import { once } from 'node:events'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import { Worker } from 'node:worker_threads'
import type { ReviewFileDiffResponse } from '../../api/types.generated'
import { startAbortableResourceAttempt } from '../../lib/use-cached-resource'
import {
  REVIEW_FILE_DIFF_RENDER_BUDGET,
  type ReviewFileDiffWorkerInput,
  reviewFileTextMetrics,
} from './review-file-diff-render-contract'
import {
  boundedText,
  createReviewFileDiffRenderer,
  ReviewDiffTransientError,
  runReviewFileDiffWorker,
} from './review-file-diff-renderer'

function textDiff(oldText: string, newText: string): ReviewFileDiffResponse {
  return {
    kind: 'Modified',
    new_content: { kind: 'text', text: newText },
    new_mode: '100644',
    old_content: { kind: 'text', text: oldText },
    old_mode: '100644',
    path: '/fixture.ts',
  }
}

test('counts UTF-8 bytes, lines, and the longest line exactly', () => {
  assert.deepEqual(reviewFileTextMetrics('a😀\nβ'), {
    bytes: 8,
    lines: 2,
    maxLineBytes: 5,
  })
  assert.deepEqual(reviewFileTextMetrics(''), {
    bytes: 0,
    lines: 0,
    maxLineBytes: 0,
  })
})

test('rejects input and line amplification before worker admission', async () => {
  let renders = 0
  const render = createReviewFileDiffRenderer({
    isolatedRender: async () => {
      renders += 1
      return { kind: 'empty' }
    },
  })

  const bytes = await render(textDiff('', 'x'.repeat(
    REVIEW_FILE_DIFF_RENDER_BUDGET.maxInputBytes + 1,
  )))
  assert.deepEqual(bytes.presentation, { kind: 'omitted', reason: 'input' })

  const longLine = await render(textDiff('', 'x'.repeat(
    REVIEW_FILE_DIFF_RENDER_BUDGET.maxInputLineBytes + 1,
  )))
  assert.deepEqual(longLine.presentation, { kind: 'omitted', reason: 'input' })

  const lines = await render(textDiff('', 'x\n'.repeat(10_000)))
  assert.deepEqual(lines.presentation, { kind: 'omitted', reason: 'lines' })
  assert.equal(renders, 0)
})

test('bounds mixed content without returning raw transport fields', async () => {
  const render = createReviewFileDiffRenderer({
    isolatedRender: async () => assert.fail('mixed content must not enter Pierre'),
  })
  const source = '😀'.repeat(REVIEW_FILE_DIFF_RENDER_BUDGET.maxMixedTextBytes)
  const result = await render({
    kind: 'Modified',
    new_content: { kind: 'text', text: source },
    new_mode: '100644',
    old_content: { kind: 'binary', oid: 'abc123', size_bytes: 42 },
    old_mode: '100644',
    path: '/fixture.dat',
  })

  assert.equal(result.presentation.kind, 'mixed')
  assert.equal('old_content' in result, false)
  assert.equal('new_content' in result, false)
  if (result.presentation.kind !== 'mixed') return
  assert.equal(result.presentation.text[0]?.truncated, true)
  assert.ok(
    Buffer.byteLength(result.presentation.text[0]?.content ?? '') <=
      REVIEW_FILE_DIFF_RENDER_BUDGET.maxMixedTextBytes,
  )
  assert.ok(
    Buffer.byteLength(JSON.stringify(result)) <
      REVIEW_FILE_DIFF_RENDER_BUDGET.maxMixedTextBytes + 1_024,
  )
})

test('admits no queue and allows retry after a transient busy failure', async () => {
  const releases: Array<() => void> = []
  const state = { active: 0 }
  const render = createReviewFileDiffRenderer({
    isolatedRender: () => new Promise((resolveRender) => {
      releases.push(() => resolveRender({ kind: 'empty' }))
    }),
    state,
  })

  const first = render(textDiff('a', 'b'))
  const second = render(textDiff('c', 'd'))
  await assert.rejects(
    render(textDiff('e', 'f')),
    (error: unknown) => error instanceof ReviewDiffTransientError &&
      error.failure === 'busy',
  )
  assert.equal(releases.length, 2)

  releases.shift()?.()
  await first
  const retry = render(textDiff('e', 'f'))
  assert.equal(releases.length, 2)
  releases.shift()?.()
  releases.shift()?.()
  await Promise.all([second, retry])
  assert.equal(state.active, 0)
})

test('terminates a CPU-bound worker at the deadline', async () => {
  const worker = new Worker('while (true) {}', { eval: true })
  const exited = once(worker, 'exit')
  const started = performance.now()
  await assert.rejects(
    runReviewFileDiffWorker(worker, 25),
    (error: unknown) => error instanceof ReviewDiffTransientError &&
      error.failure === 'deadline',
  )
  await exited
  assert.ok(performance.now() - started < 1_000)
})

test('does not publish transient busy or deadline failures to a cache', async () => {
  const busyRender = createReviewFileDiffRenderer({
    isolatedRender: async () => assert.fail('busy render must not start'),
    state: { active: REVIEW_FILE_DIFF_RENDER_BUDGET.maxConcurrentRenders },
  })
  const deadlineRender = createReviewFileDiffRenderer({
    isolatedRender: async () => {
      throw new ReviewDiffTransientError('deadline')
    },
  })

  await assertTransientNotPublished(() => busyRender(textDiff('a', 'b')), 'busy')
  await assertTransientNotPublished(
    () => deadlineRender(textDiff('a', 'b')),
    'deadline',
  )
})

test('settles the task outcome even if worker termination rejects', async () => {
  const listeners = new Map<string, (...args: never[]) => void>()
  const worker = {
    once(event: string, listener: (...args: never[]) => void) {
      listeners.set(event, listener)
      return this
    },
    terminate: async () => {
      throw new Error('already stopped')
    },
  }
  const result = runReviewFileDiffWorker(
    worker as unknown as Parameters<typeof runReviewFileDiffWorker>[0],
    100,
  )
  listeners.get('message')?.({ kind: 'empty' } as never)
  assert.deepEqual(await result, { kind: 'empty' })
})

test('zero-hunk and adversarial fixtures return bounded worker results', async () => {
  assert.deepEqual(
    await runSourceWorker(workerInput('same\n', 'same\n')),
    { kind: 'empty' },
  )
  assert.deepEqual(
    await runSourceWorker(workerInput('a\n', 'b\n', {
      maxHighlightLanguages: 0,
    })),
    { kind: 'error' },
  )
  assert.deepEqual(
    await runSourceWorker(workerInput('a\n', 'b\n', { maxHunks: 0 })),
    { kind: 'omitted', reason: 'hunks' },
  )
  assert.deepEqual(
    await runSourceWorker(workerInput('a\n', 'b\n', { maxRenderedLines: 1 })),
    { kind: 'omitted', reason: 'lines' },
  )
  assert.deepEqual(
    await runSourceWorker(workerInput('a\n', 'b\n', { maxOutputBytes: 100 })),
    { kind: 'omitted', reason: 'output' },
  )

  const thousandOld = Array.from(
    { length: 1_000 },
    (_, index) => `export const before${index} = "aaaaaaaaaaaa"\n`,
  ).join('')
  const thousandNew = Array.from(
    { length: 1_000 },
    (_, index) => `export const after${index} = "bbbbbbbbbbbb"\n`,
  ).join('')
  assert.deepEqual(
    await runSourceWorker(workerInput(thousandOld, thousandNew)),
    { kind: 'omitted', reason: 'lines' },
  )
})

test('bounds text excerpts without splitting UTF-8 characters', () => {
  assert.deepEqual(boundedText('a😀b', 5), {
    content: 'a😀',
    truncated: true,
  })
})

function workerInput(
  oldText: string,
  newText: string,
  budgetOverrides: Partial<ReviewFileDiffWorkerInput['budget']> = {},
): ReviewFileDiffWorkerInput {
  return {
    budget: {
      maxHighlightLanguages: REVIEW_FILE_DIFF_RENDER_BUDGET.maxHighlightLanguages,
      maxHunks: REVIEW_FILE_DIFF_RENDER_BUDGET.maxHunks,
      maxOutputBytes: REVIEW_FILE_DIFF_RENDER_BUDGET.maxOutputBytes,
      maxRenderedLines: REVIEW_FILE_DIFF_RENDER_BUDGET.maxRenderedLines,
      ...budgetOverrides,
    },
    newText,
    oldText,
    path: '/fixture.ts',
  }
}

function runSourceWorker(input: ReviewFileDiffWorkerInput) {
  const workerPath = resolve(
    process.cwd(),
    'src/features/review/review-file-diff-render-worker.ts',
  )
  return runReviewFileDiffWorker(new Worker(pathToFileURL(workerPath), {
    workerData: input,
  }), REVIEW_FILE_DIFF_RENDER_BUDGET.deadlineMs)
}

function assertTransientNotPublished(
  load: () => Promise<object>,
  expectedFailure: 'busy' | 'deadline',
) {
  return new Promise<void>((resolveAttempt, rejectAttempt) => {
    startAbortableResourceAttempt({
      load: async () => load(),
      onFailed: (error) => {
        try {
          assert.ok(error instanceof ReviewDiffTransientError)
          assert.equal(error.failure, expectedFailure)
          resolveAttempt()
        } catch (assertionError) {
          rejectAttempt(assertionError)
        }
      },
      onLoaded: () => rejectAttempt(
        new Error('transient failures must not be published to the cache'),
      ),
    })
  })
}
