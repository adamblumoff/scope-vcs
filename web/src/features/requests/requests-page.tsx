import type {
  RepoLiveState,
  RepoParams,
  RequestList,
  RequestListItem,
} from '@/api/types'
import { RepoShell } from '@/components/repo-shell'
import { SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import { ArrowRight, Coins, GitBranch, GitPullRequest } from 'lucide-react'
import { useState } from 'react'
import { appendRequestPage, requestCountLabel } from './request-list-model'
import {
  formatUnixDate,
  requestAuthorRoleLabel,
  requestAudienceLabel,
  requestMergeabilityLabel,
  requestStatusLabel,
  requestStatusTone,
  shortOid,
} from './request-labels'

export function RequestsPage({
  initialPage,
  loadNextPage,
  params,
}: {
  initialPage: RequestList
  live: RepoLiveState
  loadNextPage: (cursor: string) => Promise<RequestList>
  params: RepoParams
}) {
  const [loadedPages, setLoadedPages] = useState<RequestList[]>([])
  const requests = loadedPages.reduce(
    (current, page) => appendRequestPage(current, page.requests),
    initialPage.requests,
  )
  const nextCursor =
    loadedPages.length > 0
      ? loadedPages[loadedPages.length - 1].next_cursor
      : initialPage.next_cursor
  const [loadingMore, setLoadingMore] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)

  async function loadMore() {
    if (!nextCursor || loadingMore) {
      return
    }
    setLoadingMore(true)
    setLoadError(null)
    try {
      const page = await loadNextPage(nextCursor)
      setLoadedPages((current) => [...current, page])
    } catch (error) {
      setLoadError(
        error instanceof Error ? error.message : 'Could not load more requests.',
      )
    } finally {
      setLoadingMore(false)
    }
  }

  return (
    <RepoShell params={params}>
      <WorkbenchHeader
        count={requestCountLabel(requests.length, Boolean(nextCursor))}
        description="Review and settle proposed branch updates."
        eyebrow="Review"
        title="Requests"
      />
      <div className="px-4 pb-10 sm:px-6 lg:px-8">
        {requests.length > 0 ? (
          <SectionRows className="mt-4">
            {requests.map((request) => (
              <RequestListRow
                key={request.id}
                params={params}
                request={request}
              />
            ))}
            {nextCursor ? (
              <div className="px-3 py-5 text-center">
                <Button
                  disabled={loadingMore}
                  onClick={() => void loadMore()}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  {loadingMore ? 'Loading…' : 'Load more requests'}
                </Button>
                {loadError ? (
                  <p className="mt-2 text-sm text-destructive" role="alert">
                    {loadError}
                  </p>
                ) : null}
              </div>
            ) : null}
          </SectionRows>
        ) : (
          <EmptyRequests params={params} />
        )}
      </div>
    </RepoShell>
  )
}

function RequestListRow({
  params,
  request,
}: {
  params: RepoParams
  request: RequestListItem
}) {
  return (
    <Link
      className="group grid min-w-0 gap-3 px-3 py-5 transition-colors hover:bg-muted/45 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start"
      params={{ ...params, requestId: request.id }}
      to="/repos/$owner/$repo/requests/$requestId"
    >
      <div className="flex min-w-0 items-start gap-3">
        <GitPullRequest className="mt-1 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold leading-6 tracking-[-0.012em] group-hover:underline">
            {request.name}
          </h2>
          <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted-foreground">
            {request.title !== request.name ? (
              <>
                <span>{request.title}</span>
                <MetadataSeparator />
              </>
            ) : null}
            <span className="font-mono">{request.id}</span>
            <MetadataSeparator />
            <span>{requestAudienceLabel(request)}</span>
            <MetadataSeparator />
            <span>{requestAuthorRoleLabel(request)}</span>
            <MetadataSeparator />
            <span className="font-mono tabular-nums">
              {shortOid(request.head_oid)}
            </span>
            <MetadataSeparator />
            <span className="tabular-nums">
              Updated {formatUnixDate(request.updated_at_unix)}
            </span>
            {request.current_stake_credits > 0 ? (
              <>
                <MetadataSeparator />
                <span className="inline-flex items-center gap-1 tabular-nums">
                  <Coins className="size-3" />
                  {request.current_stake_credits} staked
                </span>
              </>
            ) : null}
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2 pl-7 sm:justify-end sm:pl-0">
        <Badge variant={requestStatusTone(request)}>
          {requestStatusLabel(request)}
        </Badge>
        <span className="text-xs text-muted-foreground">
          {requestMergeabilityLabel(request)}
        </span>
        <ArrowRight className="size-4 text-muted-foreground" />
      </div>
    </Link>
  )
}

function MetadataSeparator() {
  return <span aria-hidden="true">·</span>
}

function EmptyRequests({ params }: { params: RepoParams }) {
  return (
    <div className="mt-8 border-t border-border py-8">
      <div className="max-w-2xl">
        <div className="flex items-center gap-2 text-sm font-semibold leading-5">
          <GitBranch className="size-4 text-muted-foreground" />
          <span>No requests yet</span>
        </div>
        <p className="mt-2 text-pretty text-sm leading-6 text-muted-foreground">
          Requests are created from the CLI on a separate request branch. Start
          from a local clone, run the command below, then submit when the branch
          is ready for maintainer attention.
        </p>
        <div className="mt-4 inline-flex rounded-lg border border-border bg-muted px-3 py-2 font-mono text-xs leading-5 text-foreground">
          scope request start &lt;request-name&gt;
        </div>
        <div className="mt-4">
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/repos/$owner/$repo">
              View repository
            </Link>
          </Button>
        </div>
      </div>
    </div>
  )
}
