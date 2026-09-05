import type {
  ReviewDiffBinarySide,
  ReviewDiffPresentation,
  ReviewDiffTextSide,
  ReviewFileDiff,
} from '../../api/types'
import type {
  ReviewFileContentResponse,
  ReviewFileDiffResponse,
} from '../../api/types.generated'
import { createHash } from 'node:crypto'
import { createBoundedCache } from '../../lib/bounded-cache'
import {
  REVIEW_FILE_DIFF_RENDER_BUDGET,
  type ReviewFileDiffRenderBudget,
  type ReviewFileDiffWorkerInput,
  type ReviewFileDiffWorkerResult,
  reviewFileTextMetrics,
} from './review-file-diff-render-contract'

type ReviewDiffTransientFailure = 'busy' | 'deadline'

export class ReviewDiffTransientError extends Error {
  constructor(readonly failure: ReviewDiffTransientFailure) {
    super(failure === 'busy'
      ? 'The diff renderer is busy. Try again.'
      : 'Diff rendering took too long. Try again.')
    this.name = 'ReviewDiffTransientError'
  }
}

export type IsolatedReviewDiffRender = (
  input: ReviewFileDiffWorkerInput,
  deadlineMs: number,
  signal?: AbortSignal,
) => Promise<ReviewFileDiffWorkerResult>

export type ReviewDiffAdmissionState = { active: number }

export function createReviewFileDiffRenderer({
  budget = REVIEW_FILE_DIFF_RENDER_BUDGET,
  isolatedRender,
  state = { active: 0 },
}: {
  budget?: ReviewFileDiffRenderBudget
  isolatedRender: IsolatedReviewDiffRender
  state?: ReviewDiffAdmissionState
}) {
  const presentations = createBoundedCache<string, ReviewFileDiffWorkerResult>({
    maxEntries: 64,
    maxWeight: 8 * 1024 * 1024,
    weightOf: (result) => result.kind === 'html' ? result.html.length * 2 : 64,
  })
  const rendererIdentity = JSON.stringify(['pierre-1.2.11-v1', budget])
  return async function renderReviewFileDiff(
    diff: ReviewFileDiffResponse,
    signal?: AbortSignal,
  ): Promise<ReviewFileDiff> {
    signal?.throwIfAborted()
    const base = reviewFileDiffBase(diff)
    const contentPresentation = nonTextPresentation(diff, budget)
    if (contentPresentation) {
      return { ...base, presentation: contentPresentation }
    }

    const oldText = textContent(diff.old_content)
    const newText = textContent(diff.new_content)
    const oldMetrics = reviewFileTextMetrics(oldText)
    const newMetrics = reviewFileTextMetrics(newText)
    if (
      oldMetrics.bytes + newMetrics.bytes > budget.maxInputBytes ||
      Math.max(oldMetrics.maxLineBytes, newMetrics.maxLineBytes) >
        budget.maxInputLineBytes
    ) {
      return { ...base, presentation: { kind: 'omitted', reason: 'input' } }
    }
    if (oldMetrics.lines + newMetrics.lines > budget.maxInputLines) {
      return { ...base, presentation: { kind: 'omitted', reason: 'lines' } }
    }
    // Callers fetch and authorize the source before entering this renderer.
    // Cache only derived presentation; current transport metadata stays outside.
    const key = createHash('sha256')
      .update(JSON.stringify([rendererIdentity, diff.path, oldText, newText]))
      .digest('hex')
    const cached = presentations.get(key)
    if (cached && cached.kind !== 'error') return { ...base, presentation: cached }
    if (state.active >= budget.maxConcurrentRenders) {
      throw new ReviewDiffTransientError('busy')
    }

    state.active += 1
    try {
      const presentation = await isolatedRender({
        budget: {
          maxHighlightLanguages: budget.maxHighlightLanguages,
          maxHunks: budget.maxHunks,
          maxOutputBytes: budget.maxOutputBytes,
          maxRenderedLines: budget.maxRenderedLines,
        },
        newText,
        oldText,
        path: diff.path,
      }, budget.deadlineMs, signal)
      signal?.throwIfAborted()
      if (presentation.kind === 'error') {
        throw new Error('This file diff could not be rendered.')
      }
      presentations.set(key, presentation)
      return { ...base, presentation }
    } finally {
      state.active -= 1
    }
  }
}

function reviewFileDiffBase(diff: ReviewFileDiffResponse) {
  return {
    kind: diff.kind,
    new_mode: diff.new_mode,
    old_mode: diff.old_mode,
    path: diff.path,
  }
}

function nonTextPresentation(
  diff: ReviewFileDiffResponse,
  budget: ReviewFileDiffRenderBudget,
): ReviewDiffPresentation | null {
  const binary = binarySides(diff)
  if (binary.length === 0) return null

  const text = textSides(diff, budget.maxMixedTextBytes)
  return text.length > 0
    ? { binary, kind: 'mixed', text }
    : { kind: 'binary', sides: binary }
}

function binarySides(diff: ReviewFileDiffResponse): ReviewDiffBinarySide[] {
  return [
    binarySide('Old', diff.old_content),
    binarySide('New', diff.new_content),
  ].filter((side): side is ReviewDiffBinarySide => side !== null)
}

function binarySide(
  label: ReviewDiffBinarySide['label'],
  content: ReviewFileContentResponse | null,
): ReviewDiffBinarySide | null {
  return content?.kind === 'binary'
    ? { label, oid: content.oid, sizeBytes: content.size_bytes }
    : null
}

function textSides(
  diff: ReviewFileDiffResponse,
  maxBytes: number,
): ReviewDiffTextSide[] {
  return [
    textSide('Old', diff.old_content, maxBytes),
    textSide('New', diff.new_content, maxBytes),
  ].filter((side): side is ReviewDiffTextSide => side !== null)
}

function textSide(
  label: ReviewDiffTextSide['label'],
  content: ReviewFileContentResponse | null,
  maxBytes: number,
): ReviewDiffTextSide | null {
  if (content?.kind !== 'text') return null
  const { content: excerpt, truncated } = boundedText(content.text, maxBytes)
  return { content: excerpt, label, truncated }
}

export function boundedText(text: string, maxBytes: number) {
  let bytes = 0
  let content = ''
  for (const character of text) {
    const characterBytes = Buffer.byteLength(character)
    if (bytes + characterBytes > maxBytes) {
      return { content, truncated: true }
    }
    bytes += characterBytes
    content += character
  }
  return { content, truncated: false }
}

function textContent(content: ReviewFileContentResponse | null) {
  return content?.kind === 'text' ? content.text : ''
}
