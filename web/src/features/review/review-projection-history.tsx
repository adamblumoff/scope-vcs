import type { ProjectionPreview, ProjectionPreviewCommit } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ChevronDown, ChevronUp, GitCommit } from 'lucide-react'
import { useState } from 'react'
import {
  audienceLabel,
  changeCountLabel,
  commitCountLabel,
  olderCommitLabel,
} from './review-labels'

export function ReviewProjectionHistory({
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
