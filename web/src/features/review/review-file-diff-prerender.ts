import type { ReviewFileDiff } from '@/api/types'
import { parseDiffFromFile, type FileDiffOptions } from '@pierre/diffs'
import { preloadFileDiff } from '@pierre/diffs/ssr'

const REVIEW_FILE_DIFF_OPTIONS = {
  diffStyle: 'unified',
  disableFileHeader: true,
  hunkSeparators: 'line-info-basic',
  lineDiffType: 'word',
  overflow: 'wrap',
} satisfies FileDiffOptions<undefined>

export async function prerenderReviewFileDiff(
  diff: ReviewFileDiff,
): Promise<string | null> {
  const oldText = reviewFileText(diff.old_content)
  const newText = reviewFileText(diff.new_content)

  if (oldText === null || newText === null) {
    return null
  }

  const fileDiff = parseDiffFromFile(
    {
      contents: oldText,
      name: diff.path,
    },
    {
      contents: newText,
      name: diff.path,
    },
  )

  const { prerenderedHTML } = await preloadFileDiff({
    fileDiff,
    options: REVIEW_FILE_DIFF_OPTIONS,
  })

  return prerenderedHTML
}

function reviewFileText(content: ReviewFileDiff['old_content']) {
  if (!content) {
    return ''
  }

  return content.kind === 'text' ? content.text : null
}
