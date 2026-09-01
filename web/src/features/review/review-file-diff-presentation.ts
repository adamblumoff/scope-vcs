import type { ReviewDiffOmittedReason, ReviewFileDiff } from '../../api/types'

export function reviewFileDiffEmptyLabel(diff: ReviewFileDiff | null) {
  const modeChange = reviewFileDiffModeChangeLabel(diff)
  if (modeChange) return modeChange
  if (diff?.kind === 'Added') return 'Empty file added'
  if (diff?.kind === 'Deleted') return 'Empty file deleted'
  return 'No content changes'
}

export function reviewFileDiffModeChangeLabel(diff: ReviewFileDiff | null) {
  if (!diff?.old_mode || !diff.new_mode || diff.old_mode === diff.new_mode) {
    return null
  }
  return `Mode ${diff.old_mode} → ${diff.new_mode}`
}

export function reviewFileDiffOmittedLabel(reason: ReviewDiffOmittedReason) {
  return reason === 'output'
    ? 'Rendered diff is too large to display'
    : 'Diff is too large to render'
}
