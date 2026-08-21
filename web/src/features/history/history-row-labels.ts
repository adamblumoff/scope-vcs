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
  const fileCount = `${entry.change_count} ${entry.change_count === 1 ? 'file' : 'files'}`
  return {
    ariaLabel: `${kind}: ${title}, update ${entry.source_id}, ${fileCount}`,
    compactId: compactHistorySourceId(entry.source_id),
    kind,
    title,
  }
}

export function historyEntryKindLabel(kind: HistoryEntryKind) {
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
