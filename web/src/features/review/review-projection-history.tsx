import type {
  ProjectionPreview,
  ProjectionPreviewCommit,
  RepoParams,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ChevronDown, ChevronUp, GitCommit, History } from 'lucide-react'
import { useState } from 'react'
import {
  audienceLabel,
  changeCountLabel,
  olderCommitLabel,
} from './review-labels'

export function ReviewProjectionHistory({
  historyParams,
  preview,
}: {
  historyParams?: RepoParams
  preview: ProjectionPreview
}) {
  const [expanded, setExpanded] = useState(false)
  const commits = [...preview.commits].reverse()
  const collapsedCommitCount = 1
  const visibleCommits = expanded
    ? commits
    : commits.slice(0, collapsedCommitCount)
  const olderCount = Math.max(commits.length - visibleCommits.length, 0)

  return (
    <div className="mt-8 border-y border-border px-2 py-4">
      <div className="mb-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <Badge className="border-border bg-background text-foreground">
              <History className="size-3.5" />
              {audienceLabel(preview.audience)} history
            </Badge>
          </div>
          <h3 className="mt-2 flex items-center gap-2 text-base font-semibold leading-6">
            <GitCommit className="size-4 text-muted-foreground" />
            <span>History in {audienceLabel(preview.audience)} view</span>
          </h3>
        </div>
      </div>

      {visibleCommits.length === 0 ? (
        <div className="border-y border-border py-6 text-sm text-muted-foreground">
          No visible history for this view.
        </div>
      ) : (
        <div className="relative mt-4 space-y-2 pl-7">
          <div className="absolute bottom-3 left-2.5 top-3 w-px bg-border" />
          {visibleCommits.map((commit, index) => (
            <HistoryCommitRow
              commit={commit}
              historyParams={historyParams}
              index={index}
              key={commit.projected_id}
              preview={preview}
            />
          ))}
        </div>
      )}

      {olderCount > 0 && (
        <div className="mt-2 pl-7">
          <Button
            className="h-8 border-border bg-background px-3 text-sm text-foreground shadow-sm hover:bg-muted"
            onClick={() => setExpanded(true)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <ChevronDown className="size-3.5 text-muted-foreground" />
            <span>{olderCommitLabel(olderCount)}</span>
          </Button>
        </div>
      )}
      {expanded && commits.length > collapsedCommitCount && (
        <div className="mt-2 pl-7">
          <Button
            className="h-8 border-border bg-background px-3 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={() => setExpanded(false)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <ChevronUp className="size-3.5 text-muted-foreground" />
            <span>Show latest commit</span>
          </Button>
        </div>
      )}
    </div>
  )
}

function HistoryCommitRow({
  commit,
  historyParams,
  index,
  preview,
}: {
  commit: ProjectionPreviewCommit
  historyParams?: RepoParams
  index: number
  preview: ProjectionPreview
}) {
  const content = <HistoryCommitContent commit={commit} index={index} />

  return (
    <div className="relative">
      <div className="absolute -left-[23px] top-4 flex size-5 items-center justify-center rounded-full border border-border bg-background shadow-sm">
        <span className="size-2 rounded-full bg-foreground" />
      </div>
      {historyParams ? (
        <Link
          className="grid items-center gap-1 border border-border bg-background px-3 py-2 text-sm shadow-sm transition-colors hover:bg-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background sm:grid-cols-[minmax(0,1fr)_auto]"
          params={historyParams}
          search={{
            audience: preview.audience,
            commit: commit.projected_id,
          }}
          to="/repos/$owner/$repo/history"
        >
          {content}
        </Link>
      ) : (
        <div className="grid items-center gap-1 border border-border bg-background px-3 py-2 text-sm shadow-sm sm:grid-cols-[minmax(0,1fr)_auto]">
          {content}
        </div>
      )}
    </div>
  )
}

function HistoryCommitContent({
  commit,
  index,
}: {
  commit: ProjectionPreviewCommit
  index: number
}) {
  return (
    <>
      <div className="min-w-0">
        <div
          className="truncate font-mono text-xs leading-4"
          title={commit.message}
        >
          {commit.message}
        </div>
        <div className="mt-0.5 flex flex-wrap gap-2 text-xs leading-4 text-muted-foreground">
          <span>#{index + 1}</span>
          <span>{changeCountLabel(commit.change_count)}</span>
          {commit.author && <span>{commit.author}</span>}
        </div>
      </div>
      <div className="sm:text-right">
        <CommitVisibilityBadge visibility={commit.visibility} />
      </div>
    </>
  )
}

function CommitVisibilityBadge({
  visibility,
}: {
  visibility: ProjectionPreviewCommit['visibility']
}) {
  if (visibility === 'FullyPrivate') {
    return (
      <Badge className="border-red-400 bg-red-100 text-red-900">
        Fully private
      </Badge>
    )
  }

  if (visibility === 'Synthetic') {
    return (
      <Badge className="border-yellow-500 bg-yellow-100 text-yellow-900">
        Synthetic
      </Badge>
    )
  }

  if (visibility === 'Mixed') {
    return (
      <Badge className="border-yellow-500 bg-yellow-100 text-yellow-900">
        Mixed
      </Badge>
    )
  }

  return (
    <Badge className="border-green-500 bg-green-100 text-green-900">
      Fully public
    </Badge>
  )
}
