import { parseDiffFromFile, type FileDiffOptions, type SupportedLanguages } from '@pierre/diffs'
import { preloadFileDiff } from '@pierre/diffs/ssr'
import { parentPort } from 'node:worker_threads'
import type {
  ReviewFileDiffWorkerInput,
  ReviewFileDiffWorkerResult,
} from './review-file-diff-render-contract'

const REVIEW_DIFF_LANGUAGE_BY_EXTENSION = Object.freeze({
  css: 'css',
  go: 'go',
  html: 'html',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsx: 'jsx',
  md: 'markdown',
  py: 'python',
  rs: 'rust',
  sh: 'shellscript',
  ts: 'typescript',
  tsx: 'tsx',
  yaml: 'yaml',
  yml: 'yaml',
} as const)

const REVIEW_FILE_DIFF_OPTIONS = {
  diffStyle: 'unified',
  disableFileHeader: true,
  hunkSeparators: 'line-info-basic',
  lineDiffType: 'word',
  maxLineDiffLength: 512,
  overflow: 'wrap',
  tokenizeMaxLineLength: 1_000,
} satisfies FileDiffOptions<undefined>

async function renderReviewFileDiff(
  input: ReviewFileDiffWorkerInput,
): Promise<ReviewFileDiffWorkerResult> {
  if (
    Object.keys(REVIEW_DIFF_LANGUAGE_BY_EXTENSION).length >
    input.budget.maxHighlightLanguages
  ) {
    return { kind: 'error' }
  }
  const language = reviewDiffLanguage(input.path)
  const fileDiff = parseDiffFromFile(
    { contents: input.oldText, lang: language, name: input.path },
    { contents: input.newText, lang: language, name: input.path },
  )

  if (fileDiff.hunks.length === 0) {
    return { kind: 'empty' }
  }
  if (fileDiff.hunks.length > input.budget.maxHunks) {
    return { kind: 'omitted', reason: 'hunks' }
  }
  if (fileDiff.unifiedLineCount > input.budget.maxRenderedLines) {
    return { kind: 'omitted', reason: 'lines' }
  }

  const { prerenderedHTML } = await preloadFileDiff({
    fileDiff,
    options: {
      ...REVIEW_FILE_DIFF_OPTIONS,
      tokenizeMaxLength: input.budget.maxRenderedLines,
    },
  })
  if (Buffer.byteLength(prerenderedHTML) > input.budget.maxOutputBytes) {
    return { kind: 'omitted', reason: 'output' }
  }
  return { html: prerenderedHTML, kind: 'html' }
}

function reviewDiffLanguage(path: string): SupportedLanguages {
  const extension = path.split('.').at(-1)?.toLowerCase() ?? ''
  return REVIEW_DIFF_LANGUAGE_BY_EXTENSION[
    extension as keyof typeof REVIEW_DIFF_LANGUAGE_BY_EXTENSION
  ] ?? 'text'
}

const port = parentPort
if (!port) {
  throw new Error('Review diff renderer must run in a worker thread')
}

port.on('message', (input: ReviewFileDiffWorkerInput) => {
  renderReviewFileDiff(input).then(
    (result) => port.postMessage(result),
    () => port.postMessage({ kind: 'error' } satisfies ReviewFileDiffWorkerResult),
  )
})
