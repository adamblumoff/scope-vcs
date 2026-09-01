import type { CommitFile, HistoryEntryDetail } from '@/api/types'
import { PanelState, EmptyState } from '@/components/empty-state'
import { FileSystemTree } from '@/components/file-system-tree'
import { PendingSurface } from '@/components/pending-surface'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { historyCommitTitle } from '@/features/history/history-row-labels'
import type {
  CommitDetailState,
  CommitFileDiffState,
} from '@/features/history/history-state'
import { GitCommit, TriangleAlert } from 'lucide-react'
import { type ReactNode, useRef } from 'react'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'
import { VisibilityChanges } from './history-visibility-changes'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'

const EMPTY_VISIBILITY_CHANGES: HistoryEntryDetail['visibility_changes'] = []

type CommitDetailPanelProps = {
  commitContext?: ReactNode
  commitState: CommitDetailState
  diffIdentity: string | null
  diffScrollTop: number
  fileDiffState: CommitFileDiffState
  onCloseDiff: () => void
  onDiffScroll: (scrollTop: number) => void
  onRetryCommit?: () => void
  onRetryDiff?: () => void
  onSelectFile: (file: CommitFile) => void
  selectedFilePath: string | null
  terminology?: 'commit' | 'update'
  visibilityChanges?: HistoryEntryDetail['visibility_changes']
}

export function CommitDetailPanel(props: CommitDetailPanelProps) {
  return <CommitDetailPanelContent {...props} />
}

function CommitDetailPanelContent({
  commitContext,
  commitState,
  diffIdentity,
  diffScrollTop,
  fileDiffState,
  onCloseDiff,
  onDiffScroll,
  onRetryCommit,
  onRetryDiff,
  onSelectFile,
  selectedFilePath,
  terminology = 'commit',
  visibilityChanges = EMPTY_VISIBILITY_CHANGES,
}: CommitDetailPanelProps) {
  const fileNavigatorRef = useRef<HTMLDivElement>(null)

  if (commitState.status === 'loading') {
    return (
      <PendingSurface
        className="min-h-[340px]"
        delay
        label={`Loading ${terminology} details`}
      >
        <CommitDetailSkeleton showDiff={selectedFilePath !== null} />
      </PendingSurface>
    )
  }

  if (commitState.status === 'failed') {
    return (
      <PanelState tone="error">
        <TriangleAlert className="size-5" />
        <span>{commitState.error}</span>
        {onRetryCommit && (
          <Button onClick={onRetryCommit} size="sm" type="button" variant="secondary">
            Retry
          </Button>
        )}
      </PanelState>
    )
  }

  if (!commitState.commit) {
    return (
      <PanelState>
        <GitCommit className="size-5" />
        <span>Select {terminology === 'update' ? 'an' : 'a'} {terminology}</span>
      </PanelState>
    )
  }

  const commit = commitState.commit
  const diffOpen = selectedFilePath !== null
  const filesTruncated = commit.files_truncated
  function closeDiff() {
    onCloseDiff()
    requestAnimationFrame(() => fileNavigatorRef.current?.focus())
  }

  return (
    <div className="scope-content-enter min-w-0">
      <div className="border-b border-border px-5 py-4 sm:px-6">
        <h3 className="truncate text-sm font-semibold leading-5">
          {historyCommitTitle(commit)}
        </h3>
        <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-xs text-muted-foreground">
          <span>{commit.logical_commit_id}</span>
          {commit.author && (
            <>
              <span aria-hidden>·</span>
              <span>{commit.author}</span>
            </>
          )}
        </div>
        {commitContext}
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]">
        <div
          aria-label={`${capitalize(terminology)} file navigator`}
          className="min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          ref={fileNavigatorRef}
          tabIndex={-1}
        >
          {visibilityChanges.length > 0 ? (
            <VisibilityChanges changes={visibilityChanges} />
          ) : null}
          {commit.files.length === 0 && visibilityChanges.length === 0 ? (
            <EmptyState
              inline
              className="px-5 py-8 sm:px-6"
              title={filesTruncated
                ? `${commit.change_count} changed files are outside the bounded file list.`
                : `No file changes in this ${terminology}.`}
            />
          ) : commit.files.length > 0 ? (
            <FileSystemTree
              compactVisibility
              files={commit.files}
              getFileMeta={commitFileStatus}
              metaColumnLabel="Change"
              onSelectFile={onSelectFile}
              selectedFilePath={selectedFilePath}
            />
          ) : null}
        </div>
        <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden border-border xl:border-l">
          {diffOpen ? (
            <ReviewFileDiffDrawer
              cacheKey={diffIdentity}
              diff={fileDiffState.diff}
              error={fileDiffState.error}
              loading={fileDiffState.status === 'loading'}
              onClose={closeDiff}
              onRetry={fileDiffState.status === 'failed' ? onRetryDiff : undefined}
              onScrollTopChange={onDiffScroll}
              scrollTop={diffScrollTop}
              selectedPath={selectedFilePath}
            />
          ) : commit.files.length === 0 && visibilityChanges.length > 0 ? (
            <PanelState>
              <span>Visibility changes do not have a content diff</span>
            </PanelState>
          ) : commit.files.length === 0 && filesTruncated ? (
            <PanelState>
              <span>Changed files are outside the bounded file list</span>
            </PanelState>
          ) : (
            <PanelState>
              <span>Select a changed file</span>
            </PanelState>
          )}
        </div>
      </div>
    </div>
  )
}

function commitFileStatus(file: CommitFile) {
  return <Badge variant="neutral">{file.kind}</Badge>
}

function capitalize(value: string) {
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`
}
