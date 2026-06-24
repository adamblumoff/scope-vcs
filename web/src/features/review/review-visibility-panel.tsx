import type {
  ProjectionPreviewAudience,
  ProjectionPreviews,
  RepoParams,
  ReviewFileDiff,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { cn } from '@/lib/utils'
import { lazy, Suspense, useMemo, useState } from 'react'
import { ReviewEmptyFiles } from './review-empty-files'
import { ReviewProjectionHistory } from './review-projection-history'
import { ReviewTree, type ReviewTreeVariant } from './review-tree'
import { displayPath } from './review-tree-model'
import { ReviewVisibilityPanelHeader } from './review-visibility-panel-header'

const ReviewFileDiffDrawer = lazy(() =>
  import('./review-file-diff-drawer').then((module) => ({
    default: module.ReviewFileDiffDrawer,
  })),
)

export function ReviewVisibilityPanel({
  description = 'Set public or private access in the tree. Switch views to see which rows that audience receives.',
  disabled = false,
  emptyDescription,
  emptyTitle = 'No files found',
  files,
  historyParams,
  onSetVisibility,
  onCloseFileDiff,
  onSelectAudience,
  onSelectFile,
  pendingKey,
  preferredAudience,
  selectedFileDiff = null,
  selectedFileDiffError = null,
  selectedFileDiffLoading = false,
  selectedFilePath = null,
  previews,
  stagedReview = false,
  title = 'Visibility',
  treeVariant = 'workflow',
}: {
  description?: string
  disabled?: boolean
  emptyDescription?: string
  emptyTitle?: string
  files: ReviewFile[]
  historyParams?: RepoParams
  onSetVisibility?: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  onCloseFileDiff?: () => void
  onSelectAudience?: (audience: ProjectionPreviewAudience) => void
  onSelectFile?: (file: ReviewFile) => void
  pendingKey: string | null
  preferredAudience?: ProjectionPreviewAudience
  selectedFileDiff?: ReviewFileDiff | null
  selectedFileDiffError?: string | null
  selectedFileDiffLoading?: boolean
  selectedFilePath?: string | null
  previews: ProjectionPreviews
  stagedReview?: boolean
  title?: string
  treeVariant?: ReviewTreeVariant
}) {
  const [internalPreferredAudience, setInternalPreferredAudience] =
    useState<ProjectionPreviewAudience>('public')
  const availableAudiences = useMemo(
    () =>
      [
        previews.owner ? 'owner' : null,
        previews.public ? 'public' : null,
      ].filter(Boolean) as ProjectionPreviewAudience[],
    [previews.owner, previews.public],
  )

  const selectedAudience = preferredAudience ?? internalPreferredAudience
  const audience = availableAudiences.includes(selectedAudience)
    ? selectedAudience
    : availableAudiences[0]
  const preview = audience ? previews[audience] : null
  const handleSelectAudience = onSelectAudience ?? setInternalPreferredAudience
  const visiblePaths = useMemo(
    () => new Set((preview?.files ?? []).map((file) => displayPath(file.path))),
    [preview?.files],
  )
  if (!preview) {
    return null
  }
  const diffOpen = selectedFilePath !== null
  const projectionHistoryParams =
    preview.source === 'live' ? historyParams : undefined

  return (
    <section className="mt-8 border-y border-border">
      <ReviewVisibilityPanelHeader
        audience={preview.audience}
        availableAudiences={availableAudiences}
        description={description}
        onSelectAudience={handleSelectAudience}
        source={previews.source}
        title={title}
      />

      <div className="border-b border-border py-4">
        {files.length === 0 ? (
          <ReviewEmptyFiles
            description={
              emptyDescription ??
              (stagedReview
                ? 'No staged push is waiting.'
                : 'This repo can still be published.')
            }
            title={emptyTitle}
          />
        ) : (
          <div
            className={cn(
              'mt-4 grid grid-cols-1 transition-[grid-template-columns] duration-300 ease-out lg:mt-5',
              diffOpen
                ? 'lg:grid-cols-[minmax(0,0.95fr)_minmax(360px,1.05fr)]'
                : 'lg:grid-cols-[minmax(0,1fr)_minmax(0,0fr)]',
            )}
          >
            <div className="min-w-0">
              <ReviewTree
                audience={preview.audience}
                disabled={disabled}
                files={files}
                onSelectFile={onSelectFile}
                onSetVisibility={onSetVisibility}
                pendingKey={pendingKey}
                selectedFilePath={selectedFilePath}
                stagedReview={stagedReview}
                visiblePaths={visiblePaths}
                variant={treeVariant}
              />
            </div>
            <div
              className={cn(
                'min-w-0 overflow-hidden border-border transition-[max-height,opacity,transform,border-color] duration-300 ease-out lg:border-l',
                diffOpen
                  ? 'mt-4 max-h-[70vh] translate-y-0 opacity-100 lg:mt-0 lg:max-h-none lg:translate-x-0'
                  : 'pointer-events-none max-h-0 -translate-y-1 border-transparent opacity-0 lg:translate-x-3',
              )}
            >
              {diffOpen ? (
                <Suspense
                  fallback={
                    <ReviewFileDiffLoadingShell selectedPath={selectedFilePath} />
                  }
                >
                  <ReviewFileDiffDrawer
                    diff={selectedFileDiff}
                    error={selectedFileDiffError}
                    loading={selectedFileDiffLoading}
                    onClose={onCloseFileDiff ?? (() => undefined)}
                    selectedPath={selectedFilePath}
                  />
                </Suspense>
              ) : null}
            </div>
          </div>
        )}
      </div>

      <div className="w-full max-w-[760px]">
        <ReviewProjectionHistory
          historyParams={projectionHistoryParams}
          preview={preview}
        />
      </div>
    </section>
  )
}

function ReviewFileDiffLoadingShell({
  selectedPath,
}: {
  selectedPath: string | null
}) {
  return (
    <div className="flex min-h-[320px] flex-col px-4 py-3 lg:px-5">
      <div className="flex min-w-0 items-center justify-between gap-3 border-b border-border pb-3">
        <div className="min-w-0">
          <div className="truncate font-mono text-sm font-medium">
            {selectedPath ?? 'Diff'}
          </div>
          <div className="mt-1 text-xs text-muted-foreground">
            Preparing diff...
          </div>
        </div>
      </div>
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading renderer...
      </div>
    </div>
  )
}
