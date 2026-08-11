import type { CommitFile, CommitSummary } from '@/api/types'
import { EmptyState } from '@/components/empty-state'
import type { WorkspaceTabItem } from '@/components/workspace-tab-model'
import { CommitDetailPanel } from '@/features/history/history-commit-detail'
import { CommitList } from '@/features/history/history-commit-list'
import {
  readHistoryDiffScroll,
  writeHistoryDiffScroll,
} from '@/features/history/history-resource-cache'
import type {
  CommitDetailState,
  CommitFileDiffState,
} from '@/features/history/history-state'
import { History } from 'lucide-react'
import type { ReactNode } from 'react'

export function HistoryWorkbench({
  commitContext,
  commitState,
  commits,
  diffIdentity,
  emptyDescription,
  emptyTitle,
  fileDiffState,
  fileTabs,
  onActivateFileTab,
  onCloseFileTab,
  onRetryCommit,
  onRetryDiff,
  onSelectCommit,
  onSelectFile,
  selectedCommitId,
  selectedFilePath,
}: {
  commitContext?: ReactNode
  commitState: CommitDetailState
  commits: CommitSummary[]
  diffIdentity: string | null
  emptyDescription: string
  emptyTitle: string
  fileDiffState: CommitFileDiffState
  fileTabs: WorkspaceTabItem[]
  onActivateFileTab: (path: string) => void
  onCloseFileTab: (path: string) => string | null
  onRetryCommit?: () => void
  onRetryDiff?: () => void
  onSelectCommit: (commit: CommitSummary) => void
  onSelectFile: (file: CommitFile) => void
  selectedCommitId: string | null
  selectedFilePath: string | null
}) {
  return (
    <section className="border-t border-border">
      {commits.length === 0 ? (
        <EmptyState
          description={emptyDescription}
          icon={<History />}
          title={emptyTitle}
        />
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-[minmax(260px,0.4fr)_minmax(0,1.6fr)]">
          <CommitList
            commits={commits}
            onSelectCommit={onSelectCommit}
            selectedCommitId={selectedCommitId}
          />
          <CommitDetailPanel
            commitContext={commitContext}
            commitState={commitState}
            diffIdentity={diffIdentity}
            diffScrollTop={readHistoryDiffScroll(diffIdentity)}
            fileDiffState={fileDiffState}
            fileTabs={fileTabs}
            onActivateFileTab={onActivateFileTab}
            onCloseFileTab={onCloseFileTab}
            onDiffScroll={(scrollTop) => writeHistoryDiffScroll(diffIdentity, scrollTop)}
            onRetryCommit={onRetryCommit}
            onRetryDiff={onRetryDiff}
            onSelectFile={onSelectFile}
            selectedFilePath={selectedFilePath}
          />
        </div>
      )}
    </section>
  )
}
