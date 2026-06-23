import type {
  ProjectionPreview,
  ProjectionPreviewAudience,
  ProjectionPreviewCommit,
  ProjectionPreviewSource,
  ProjectionPreviews,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  ChevronDown,
  ChevronUp,
  FileSearch,
  GitCommit,
  Globe2,
  UserRound,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { ReviewTree, type ReviewTreeVariant } from './review-tree'
import { displayPath } from './review-tree-model'

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
      <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-sm font-semibold leading-5">{title}</h2>
            <Badge variant="outline">{sourceLabel(previews.source)}</Badge>
          </div>
          <p className="mt-1 max-w-[720px] text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        </div>
        {availableAudiences.length > 1 && (
          <AudienceToggle
            audience={preview.audience}
            availableAudiences={availableAudiences}
            onSelect={setPreferredAudience}
          />
        )}
      </div>

      <div className="border-b border-border py-4">
        <PreviewMetrics
          preview={preview}
          showPrivateCounts={showPrivateCounts}
        />
        {files.length === 0 ? (
          <EmptyFiles
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

      <ProjectionHistory
        preview={preview}
        showPrivateCounts={showPrivateCounts}
      />
    </section>
  )
}

function AudienceToggle({
  audience,
  availableAudiences,
  onSelect,
}: {
  audience: ProjectionPreviewAudience
  availableAudiences: ProjectionPreviewAudience[]
  onSelect: (audience: ProjectionPreviewAudience) => void
}) {
  return (
    <ToggleGroup
      onValueChange={(value) => {
        if (value) {
          onSelect(value as ProjectionPreviewAudience)
        }
      }}
      type="single"
      value={audience}
    >
      {(['owner', 'public'] as const).map((option) => {
        const Icon = option === 'owner' ? UserRound : Globe2
        return (
          <ToggleGroupItem
            aria-label={`${audienceLabel(option)} view`}
            disabled={!availableAudiences.includes(option)}
            key={option}
            value={option}
          >
            <Icon className="size-3" />
            <span>{audienceLabel(option)} view</span>
          </ToggleGroupItem>
        )
      })}
    </ToggleGroup>
  )
}

function PreviewMetrics({
  preview,
  showPrivateCounts,
}: {
  preview: ProjectionPreview
  showPrivateCounts: boolean
}) {
  return (
    <div className="mb-4 grid gap-3 border-y border-border py-3 text-sm sm:grid-cols-3">
      <Metric
        label="Visible"
        value={fileCountLabel(preview.summary.visible_files)}
      />
      {preview.audience === 'public' && showPrivateCounts ? (
        <Metric
          label="Private left out"
          value={fileCountLabel(preview.summary.hidden_files)}
        />
      ) : (
        <Metric
          label="Audience"
          value={audienceLabel(preview.audience)}
        />
      )}
      <Metric
        label="History"
        value={commitCountLabel(preview.summary.visible_commits)}
      />
    </div>
  )
}

function ProjectionHistory({
  preview,
  showPrivateCounts,
}: {
  preview: ProjectionPreview
  showPrivateCounts: boolean
}) {
  const [expanded, setExpanded] = useState(false)
  const commits = [...preview.commits].reverse()
  const visibleCommits = expanded ? commits : commits.slice(0, 2)
  const olderCount = Math.max(commits.length - visibleCommits.length, 0)

  return (
    <div className="mt-8 border-t border-border pb-4 pt-6">
      <div className="mb-4 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h3 className="flex items-center gap-2 text-base font-semibold leading-6">
            <GitCommit className="size-4 text-muted-foreground" />
            <span>History in {audienceLabel(preview.audience)} view</span>
          </h3>
          <p className="mt-1 text-xs leading-4 text-muted-foreground">
            {expanded
              ? `Showing all ${commitCountLabel(commits.length)}.`
              : `Showing latest ${Math.min(2, commits.length)} of ${commitCountLabel(commits.length)}.`}
            {showPrivateCounts && preview.summary.hidden_commits > 0
              ? ` ${commitCountLabel(preview.summary.hidden_commits)} left out of this view.`
              : ''}
          </p>
        </div>
      </div>

      {visibleCommits.length === 0 ? (
        <div className="border-y border-border py-6 text-sm text-muted-foreground">
          No visible history for this view.
        </div>
      ) : (
        <div className="divide-y divide-border border-y border-border">
          {visibleCommits.map((commit) => (
            <HistoryCommitRow
              commit={commit}
              key={commit.projected_id}
              showSyntheticBadge={showPrivateCounts}
            />
          ))}
        </div>
      )}

      {olderCount > 0 && (
        <Button
          className="mt-3"
          onClick={() => setExpanded(true)}
          size="sm"
          type="button"
          variant="secondary"
        >
          <ChevronDown className="size-3.5" />
          <span>{olderCommitLabel(olderCount)}</span>
        </Button>
      )}
      {expanded && commits.length > 2 && (
        <Button
          className="mt-3"
          onClick={() => setExpanded(false)}
          size="sm"
          type="button"
          variant="ghost"
        >
          <ChevronUp className="size-3.5" />
          <span>Show latest two</span>
        </Button>
      )}
    </div>
  )
}

function HistoryCommitRow({
  commit,
  showSyntheticBadge,
}: {
  commit: ProjectionPreviewCommit
  showSyntheticBadge: boolean
}) {
  return (
    <div className="grid gap-2 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto]">
      <div className="min-w-0">
        <div className="truncate font-mono text-xs leading-5">
          {commit.message}
        </div>
        <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground">
          <span>{changeCountLabel(commit.change_count)}</span>
          {commit.author && <span>{commit.author}</span>}
        </div>
      </div>
      {commit.synthetic && showSyntheticBadge && (
        <div className="sm:text-right">
          <Badge variant="outline">Public-only commit</Badge>
        </div>
      )}
    </div>
  )
}

function EmptyFiles({
  description,
  title,
}: {
  description: string
  title: string
}) {
  return (
    <div className="flex items-center gap-3 py-8">
      <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
        <FileSearch className="size-5 text-muted-foreground" />
      </div>
      <div className="min-w-0 text-sm">
        <div className="font-medium leading-5">{title}</div>
        <div className="mt-1 leading-5 text-muted-foreground">
          {description}
        </div>
      </div>
    </div>
  )
}

function Metric({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs leading-4 text-muted-foreground">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-semibold leading-5">
        {value}
      </div>
    </div>
  )
}

function audienceLabel(audience: ProjectionPreviewAudience) {
  return audience === 'owner' ? 'Owner' : 'Public'
}

function sourceLabel(source: ProjectionPreviewSource) {
  return source === 'review' ? 'After review' : 'Current repo'
}

function fileCountLabel(count: number) {
  return `${count} ${count === 1 ? 'file' : 'files'}`
}

function commitCountLabel(count: number) {
  return `${count} ${count === 1 ? 'commit' : 'commits'}`
}

function changeCountLabel(count: number) {
  return `${count} ${count === 1 ? 'change' : 'changes'}`
}

function olderCommitLabel(count: number) {
  return `Show ${count} older ${count === 1 ? 'commit' : 'commits'}`
}
