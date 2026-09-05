import { Worker } from 'node:worker_threads'
import { REVIEW_FILE_DIFF_RENDER_BUDGET } from './review-file-diff-render-contract'
import { createReviewFileDiffRenderer } from './review-file-diff-renderer'
import { createReviewFileDiffWorkerPool } from './review-file-diff-worker-pool'

const rendererKey = Symbol.for('scope.review-file-diff-renderer-v1')
const host = globalThis as unknown as Record<
  symbol,
  ReturnType<typeof createReviewFileDiffRenderer> | undefined
>

// Nitro can include this module in multiple server chunks. Share admission,
// workers and the bounded presentation cache within the process.
export const renderReviewFileDiff = host[rendererKey] ??= createReviewFileDiffRenderer({
  isolatedRender: createReviewFileDiffWorkerPool(() => new Worker(
    import.meta.env.PROD
      ? new URL('../_workers/review-file-diff-render-worker.mjs', import.meta.url)
      : new URL('./review-file-diff-render-worker.ts', import.meta.url),
  ), REVIEW_FILE_DIFF_RENDER_BUDGET.maxConcurrentRenders),
})
