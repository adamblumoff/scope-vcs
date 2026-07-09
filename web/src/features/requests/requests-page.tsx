import type { RepoLiveState, RepoParams, RequestSummary } from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight, Coins, GitBranch, GitPullRequest } from 'lucide-react'
import {
  formatUnixDate,
  requestAuthorRoleLabel,
  requestBaseAudienceLabel,
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStateLabel,
  requestStateTone,
  shortOid,
} from './request-labels'

export function RequestsPage({
  live,
  params,
  requests,
}: {
  live: RepoLiveState
  params: RepoParams
  requests: RequestSummary[]
}) {
  const { repo } = live

  return (
    <RepoShell
      active="requests"
      canManage={repo.access.actor !== 'Public'}
      params={params}
    >
      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              <Badge variant="neutral">{repo.access.actor}</Badge>
            </>
          )}
          description="Review and settle proposed branch updates."
          title="Requests"
        />

        {requests.length > 0 ? (
          <SectionRows>
            {requests.map((request) => (
              <RequestListRow
                key={request.id}
                params={params}
                request={request}
              />
            ))}
          </SectionRows>
        ) : (
          <EmptyRequests params={params} />
        )}
      </PageContent>
    </RepoShell>
  )
}

function RequestListRow({
  params,
  request,
}: {
  params: RepoParams
  request: RequestSummary
}) {
  return (
    <SectionRow
      columns="compact"
      description={
        <span className="font-mono tabular-nums">
          {shortOid(request.head_oid)} updated {formatUnixDate(request.updated_at_unix)}
        </span>
      }
      icon={<GitPullRequest className="size-4 text-muted-foreground" />}
      title={
        <span className="inline-flex min-w-0 max-w-full items-center gap-2">
          <span className="truncate">{request.id}</span>
          <Badge variant={requestStateTone(request.state)}>
            {requestStateLabel(request.state)}
          </Badge>
        </span>
      }
    >
      <div className="flex min-w-0 flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <Link
            className="block truncate text-base font-medium leading-6 hover:underline"
            params={{ ...params, requestId: request.id }}
            to="/repos/$owner/$repo/requests/$requestId"
          >
            {request.title}
          </Link>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            <Badge variant="outline">{requestBaseAudienceLabel(request)}</Badge>
            <Badge variant="outline">{requestAuthorRoleLabel(request)}</Badge>
            <Badge variant={requestMergeabilityTone(request)}>
              {requestMergeabilityLabel(request)}
            </Badge>
            <Badge variant="neutral">
              <Coins className="size-3" />
              <span>{request.stake_credits}</span>
            </Badge>
          </div>
        </div>
        <Button asChild size="sm" variant="ghost">
          <Link
            params={{ ...params, requestId: request.id }}
            to="/repos/$owner/$repo/requests/$requestId"
          >
            <span>Open</span>
            <ArrowRight className="size-3.5" />
          </Link>
        </Button>
      </div>
    </SectionRow>
  )
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
          scope request start
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
