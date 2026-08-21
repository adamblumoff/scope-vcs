import type {
  CommitSummary,
  HistoryEntryKind,
  HistoryEntrySummary,
} from '@/api/types'

type HistoryRowCommit = Pick<
  CommitSummary,
  'change_count' | 'logical_commit_id' | 'message'
>

const REVIEWED_PUSH_ID = /^rv_push_([0-9a-f]{40})$/

export function historyRowLabels(commit: HistoryRowCommit) {
  const title = historyCommitTitle(commit)
  const reviewedPush = REVIEWED_PUSH_ID.exec(commit.logical_commit_id)
  const compactId = reviewedPush
    ? reviewedPush[1].slice(0, 12)
    : commit.logical_commit_id
  const fileCount = `${commit.change_count} ${commit.change_count === 1 ? 'file' : 'files'}`

  return {
    ariaLabel: `${title}, commit ${commit.logical_commit_id}, ${fileCount}`,
    compactId,
    title,
  }
}

export function historyCommitTitle(commit: Pick<CommitSummary, 'message'>) {
  return commit.message.split(/\r?\n/, 1)[0]?.trim() || '(no message)'
}

export function historyEntryLabels(entry: HistoryEntrySummary) {
  const title = historyCommitTitle(entry)
  const kind = historyEntryKindLabel(entry.kind)
  const visibilityCount = entry.visibility_summary.made_public_count
    + entry.visibility_summary.made_private_count
  const displayedFileCount = entry.kind === 'visibility_change' ? 0 : entry.file_change_count
  const fileCount = `${displayedFileCount} ${displayedFileCount === 1 ? 'file change' : 'file changes'}`
  const visibilityCountLabel = `${visibilityCount} ${visibilityCount === 1 ? 'visibility change' : 'visibility changes'}`
  const counts = [
    displayedFileCount > 0 ? fileCount : null,
    visibilityCount > 0 ? visibilityCountLabel : null,
  ].filter(Boolean).join(', ')
  return {
    ariaLabel: `${kind}: ${title}, update ${entry.source_id}, ${counts}`,
    compactId: compactHistorySourceId(entry.source_id),
    count: displayedFileCount > 0 && visibilityCount > 0
      ? `${displayedFileCount} + ${visibilityCount}`
      : `${displayedFileCount || visibilityCount}`,
    kind,
    title,
    visibilityBreakdown: visibilityBreakdown(entry),
  }
}

function visibilityBreakdown(entry: HistoryEntrySummary) {
  const {
    made_private_count: madePrivateCount,
    made_public_count: madePublicCount,
  } = entry.visibility_summary
  if (madePublicCount > 0 && madePrivateCount > 0) {
    return `${madePublicCount} public · ${madePrivateCount} private`
  }
  if (madePublicCount > 0) return `${madePublicCount} public`
  if (madePrivateCount > 0) return `${madePrivateCount} private`
  return null
}

function historyEntryKindLabel(kind: HistoryEntryKind) {
  switch (kind) {
    case 'push':
      return 'Push'
    case 'merged_request':
      return 'Merged'
    case 'visibility_change':
      return 'Visibility'
  }
}

function compactHistorySourceId(sourceId: string) {
  const reviewedPush = REVIEWED_PUSH_ID.exec(sourceId)
  return reviewedPush ? reviewedPush[1].slice(0, 12) : sourceId
}
