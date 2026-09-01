import type { ReviewDiffOmittedReason } from '../../api/types'

export const REVIEW_FILE_DIFF_RENDER_BUDGET = Object.freeze({
  deadlineMs: 1_500,
  maxConcurrentRenders: 2,
  maxHighlightLanguages: 16,
  maxHunks: 64,
  maxInputBytes: 256 * 1024,
  maxInputLineBytes: 16 * 1024,
  maxInputLines: 4_000,
  maxMixedTextBytes: 64 * 1024,
  maxOutputBytes: 256 * 1024,
  maxRenderedLines: 800,
})

export type ReviewFileDiffRenderBudget = Readonly<Record<
  keyof typeof REVIEW_FILE_DIFF_RENDER_BUDGET,
  number
>>

export type ReviewFileDiffWorkerInput = {
  budget: Pick<
    ReviewFileDiffRenderBudget,
    'maxHighlightLanguages' | 'maxHunks' | 'maxOutputBytes' | 'maxRenderedLines'
  >
  newText: string
  oldText: string
  path: string
}

export type ReviewFileDiffWorkerResult =
  | { html: string; kind: 'html' }
  | { kind: 'empty' }
  | { kind: 'error' }
  | {
      kind: 'omitted'
      reason: Extract<ReviewDiffOmittedReason, 'hunks' | 'lines' | 'output'>
    }

export type ReviewFileTextMetrics = {
  bytes: number
  lines: number
  maxLineBytes: number
}

export function reviewFileTextMetrics(text: string): ReviewFileTextMetrics {
  const encoded = new TextEncoder().encode(text)
  if (encoded.length === 0) {
    return { bytes: 0, lines: 0, maxLineBytes: 0 }
  }

  let lineBytes = 0
  let lines = 0
  let maxLineBytes = 0
  for (const byte of encoded) {
    if (byte === 10) {
      lines += 1
      maxLineBytes = Math.max(maxLineBytes, lineBytes)
      lineBytes = 0
    } else {
      lineBytes += 1
    }
  }
  if (encoded.at(-1) !== 10) {
    lines += 1
    maxLineBytes = Math.max(maxLineBytes, lineBytes)
  }

  return { bytes: encoded.length, lines, maxLineBytes }
}
