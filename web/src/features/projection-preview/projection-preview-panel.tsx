import type {
  ProjectionPreviewAudience,
  ProjectionPreviewSource,
  ProjectionPreviews,
  RepoFile,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { GitCommit, Globe2, UserRound } from 'lucide-react'
import { useMemo, useState } from 'react'
import { ReviewTree } from '../review/review-tree'

export function ProjectionPreviewPanel({
  previews,
}: {
  previews: ProjectionPreviews
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
  if (!preview) {
    return null
  }

  const showPrivateCounts = Boolean(previews.owner)
  const treeFiles = preview.files.map((file) => ({
    oid: file.oid,
    path: file.path,
    tracked: true,
    visibility: file.visibility,
  })) satisfies RepoFile[]
  const explanation = audienceExplanation(preview.audience, previews.source)

  return (
    <section className="mt-8 border-y border-border">
      <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-sm font-semibold leading-5">
              Who can see this?
            </h2>
            <Badge variant="outline">{sourceLabel(previews.source)}</Badge>
          </div>
          <p className="mt-1 max-w-[660px] text-sm leading-5 text-muted-foreground">
            {explanation}
          </p>
        </div>
        {availableAudiences.length > 1 && (
          <div className="inline-flex w-fit items-center rounded-md border border-border p-0.5">
            <AudienceButton
              audience="owner"
              current={preview.audience}
              disabled={!previews.owner}
              onSelect={setPreferredAudience}
            />
            <AudienceButton
              audience="public"
              current={preview.audience}
              disabled={!previews.public}
              onSelect={setPreferredAudience}
            />
          </div>
        )}
      </div>

      <div className="flex flex-wrap gap-x-8 gap-y-3 border-b border-border py-4 text-sm">
        <Metric
          label="Visible in this view"
          value={fileCountLabel(preview.summary.visible_files)}
        />
        {preview.audience === 'public' && showPrivateCounts && (
          <Metric
            label="Private files left out"
            value={fileCountLabel(preview.summary.hidden_files)}
          />
        )}
        <Metric
          label="History shown"
          value={commitCountLabel(preview.summary.visible_commits)}
        />
      </div>

      {treeFiles.length === 0 ? (
        <div className="py-8 text-sm">
          <div className="font-medium leading-5">No files visible</div>
          <div className="mt-1 leading-5 text-muted-foreground">
            {audienceLabel(preview.audience)} view does not include any files.
          </div>
        </div>
      ) : (
        <ReviewTree files={treeFiles} stagedReview={false} />
      )}

      {preview.commits.length > 0 && (
        <details className="border-t border-border py-3">
          <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium leading-5">
            <GitCommit className="size-4 text-muted-foreground" />
            <span>History in this view</span>
            {showPrivateCounts && preview.summary.hidden_commits > 0 && (
              <span className="text-xs text-muted-foreground">
                {commitCountLabel(preview.summary.hidden_commits)} left out
              </span>
            )}
          </summary>
          <div className="mt-3 divide-y divide-border border-y border-border">
            {preview.commits.map((commit) => (
              <div
                className="grid gap-2 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto]"
                key={commit.projected_id}
              >
                <div className="min-w-0">
                  <div className="truncate font-mono text-xs leading-5">
                    {commit.message}
                  </div>
                  <div className="mt-1 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground">
                    <span>{changeCountLabel(commit.change_count)}</span>
                    {commit.author && <span>{commit.author}</span>}
                  </div>
                </div>
                {commit.synthetic && (
                  <div className="sm:text-right">
                    <Badge variant="outline">Public-only commit</Badge>
                  </div>
                )}
              </div>
            ))}
          </div>
        </details>
      )}
    </section>
  )
}

function AudienceButton({
  audience,
  current,
  disabled,
  onSelect,
}: {
  audience: ProjectionPreviewAudience
  current: ProjectionPreviewAudience
  disabled: boolean
  onSelect: (audience: ProjectionPreviewAudience) => void
}) {
  const selected = audience === current
  const Icon = audience === 'owner' ? UserRound : Globe2

  return (
    <Button
      aria-pressed={selected}
      className={cn('h-7 px-2', selected && 'pointer-events-none')}
      disabled={disabled}
      onClick={() => onSelect(audience)}
      size="xs"
      type="button"
      variant={selected ? 'default' : 'ghost'}
    >
      <Icon className="size-3" />
      <span>{audienceLabel(audience)} view</span>
    </Button>
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

function audienceExplanation(
  audience: ProjectionPreviewAudience,
  source: ProjectionPreviewSource,
) {
  if (audience === 'owner') {
    return source === 'review'
      ? 'Owner view shows the repo after this review is applied, including private files.'
      : 'Owner view shows the live repo exactly as you can read it, including private files.'
  }

  return source === 'review'
    ? 'Public view shows what signed-out users and public clones will receive after this review is applied.'
    : 'Public view shows what signed-out users and public clones can read right now.'
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
