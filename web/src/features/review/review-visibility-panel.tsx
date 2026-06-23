import type {
  ProjectionPreviewAudience,
  ProjectionPreviews,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { useMemo, useState } from 'react'
import { ReviewEmptyFiles } from './review-empty-files'
import { ReviewPreviewMetrics } from './review-preview-metrics'
import { ReviewProjectionHistory } from './review-projection-history'
import { ReviewTree, type ReviewTreeVariant } from './review-tree'
import { displayPath } from './review-tree-model'
import { ReviewVisibilityPanelHeader } from './review-visibility-panel-header'

export function ReviewVisibilityPanel({
  description = 'Set public or private access in the tree. Switch views to see which rows that audience receives.',
  disabled = false,
  emptyDescription,
  emptyTitle = 'No files found',
  files,
  onSetVisibility,
  pendingKey,
  previews,
  showPrivateCounts = Boolean(previews.owner),
  stagedReview = false,
  title = 'Visibility',
  treeVariant = 'workflow',
}: {
  description?: string
  disabled?: boolean
  emptyDescription?: string
  emptyTitle?: string
  files: ReviewFile[]
  onSetVisibility?: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  pendingKey: string | null
  previews: ProjectionPreviews
  showPrivateCounts?: boolean
  stagedReview?: boolean
  title?: string
  treeVariant?: ReviewTreeVariant
}) {
  const [preferredAudience, setPreferredAudience] =
    useState<ProjectionPreviewAudience>('public')
  const availableAudiences = useMemo(
    () =>
      [
        previews.owner ? 'owner' : null,
        previews.public ? 'public' : null,
      ].filter(Boolean) as ProjectionPreviewAudience[],
    [previews.owner, previews.public],
  )

  const audience = availableAudiences.includes(preferredAudience)
    ? preferredAudience
    : availableAudiences[0]
  const preview = audience ? previews[audience] : null
  const visiblePaths = useMemo(
    () => new Set((preview?.files ?? []).map((file) => displayPath(file.path))),
    [preview?.files],
  )
  if (!preview) {
    return null
  }

  return (
    <section className="mt-8 border-y border-border">
      <ReviewVisibilityPanelHeader
        audience={preview.audience}
        availableAudiences={availableAudiences}
        description={description}
        onSelectAudience={setPreferredAudience}
        source={previews.source}
        title={title}
      />

      <div className="border-b border-border py-4">
        <ReviewPreviewMetrics
          preview={preview}
          showPrivateCounts={showPrivateCounts}
        />
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
          <ReviewTree
            audience={preview.audience}
            disabled={disabled}
            files={files}
            onSetVisibility={onSetVisibility}
            pendingKey={pendingKey}
            stagedReview={stagedReview}
            visiblePaths={visiblePaths}
            variant={treeVariant}
          />
        )}
      </div>

      <ReviewProjectionHistory
        preview={preview}
        showPrivateCounts={showPrivateCounts}
      />
    </section>
  )
}
