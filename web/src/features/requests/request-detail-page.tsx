import type {
  RequestDetail,
  RepoLiveState,
  RepoParams,
  RequestMutation,
  RequestRating,
  RequestRatings,
} from '@/api/types'
import type { RateRequestInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { PageContent } from '@/components/page-header'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { GitCommit, History, MessageSquare, ShieldQuestion } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import { RequestContextRail } from './request-context-rail'
import type { RequestActivityPage } from './request-discussion-types'
import { RequestDescription } from './request-description'
import type { UpdateDescriptionInput } from './request-discussion-api'
import {
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
import { RequestLifecycleActions } from './request-lifecycle-actions'
import { useRequestActions } from './use-request-actions'
import { useRequestActivityHistory } from './use-request-activity-history'

export function RequestUnavailablePage({ params }: { params: RepoParams }) {
  return (
    <PageContent>
      <EmptyState
        action={(
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/$owner/$repo/requests">
              Back to requests
            </Link>
          </Button>
        )}
        description="It does not exist, or this account cannot see it. Sign in with an account that has access."
        icon={<ShieldQuestion />}
        title="Request not found"
      />
    </PageContent>
  )
}

type RequestDetailPageProps = {
  children: ReactNode
  detail: RequestDetail
  live: RepoLiveState
  loadActivity: () => Promise<RequestActivityPage>
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  ratings: RequestRatings
  rateRequest: (input: RateRequestInput) => Promise<RequestRating>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutation>
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    children,
    detail,
    live,
    loadActivity,
    params,
    performAction,
    ratings,
    rateRequest,
    updateDescription,
  } = props
  const { request } = detail
  const serverDescription = request.description_markdown
  const history = useRequestActivityHistory(loadActivity)
  const requestActions = useRequestActions(performAction)
  const [descriptionOverride, setDescriptionOverride] = useState<{
    server: string
    value: string
  } | null>(null)
  const description = descriptionOverride?.server === serverDescription
    ? descriptionOverride.value
    : serverDescription
  const requestParams = useMemo(() => ({
    owner: params.owner,
    repo: params.repo,
    request_id: request.id,
  }), [params.owner, params.repo, request.id])
  const hasLifecycleActions = request.permissions.can_submit ||
    request.permissions.can_merge ||
    request.permissions.can_close

  async function saveDescription(nextDescription: string) {
    try {
      await updateDescription({
        ...requestParams,
        description_markdown: nextDescription,
      })
      setDescriptionOverride({ server: serverDescription, value: nextDescription })
      return true
    } catch {
      return false
    }
  }

  function requestHeader() {
    return (
      <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <h1 className="break-words text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[30px]">
              {request.title}
            </h1>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <Badge variant={requestStatusTone(request)}>
                {requestStatusLabel(request)}
              </Badge>
              {request.state === 'Open' ? (
                <Badge variant={requestMergeabilityTone(request)}>
                  {requestMergeabilityLabel(request)}
                </Badge>
              ) : null}
              <span className="font-mono text-xs text-muted-foreground">
                {request.name}
              </span>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 xl:justify-end">
            <RequestLifecycleActions
              actions={requestActions}
              className="hidden xl:flex"
              request={request}
            />
            <Button asChild size="sm" variant="secondary">
              <Link params={params} to="/$owner/$repo/requests">Requests</Link>
            </Button>
            {request.permissions.can_view_activity ? (
              <Button
                aria-label="View request activity"
                onClick={history.openHistory}
                size="icon-sm"
                title="View request activity"
                type="button"
                variant="secondary"
              >
                <History />
              </Button>
            ) : null}
          </div>
        </div>
        {requestActions.error ? (
          <p className="mt-3 text-sm text-danger-strong" role="alert">
            {requestActions.error}
          </p>
        ) : null}
      </header>
    )
  }

  return (
    <div className={hasLifecycleActions ? 'pb-20 xl:pb-0' : undefined}>
      {requestHeader()}
      <div className="grid min-h-0 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="order-2 min-w-0 xl:order-1">
          <RequestDescription
            canEdit={request.permissions.can_edit_identity}
            description={description}
            onSave={saveDescription}
          />
          <RequestViewTabs params={{ ...params, requestId: request.id }} />
          {children}
        </div>
        <RequestContextRail
          actions={requestActions}
          onRate={rateRequest}
          params={requestParams}
          ratings={ratings}
          request={request}
        />
      </div>

      {hasLifecycleActions ? (
        <div className="fixed inset-x-0 bottom-0 z-30 border-t border-[var(--border-strong)] bg-background/95 px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur xl:hidden">
          <RequestLifecycleActions
            actions={requestActions}
            className="grid w-full grid-cols-2 [&>button]:min-h-10"
            request={request}
          />
        </div>
      ) : null}

      <RequestActivityDrawer
        activity={history.activity}
        error={history.error}
        load={history.retry}
        loading={history.loading}
        onOpenChange={history.onOpenChange}
        open={history.open}
      />
    </div>
  )
}

function RequestViewTabs({
  params,
}: {
  params: RepoParams & { requestId: string }
}) {
  const tabClass = 'inline-flex h-11 items-center gap-2 border-b-2 px-1 text-sm font-medium transition-colors'
  return (
    <nav aria-label="Request views" className="flex gap-6 border-b border-border px-5 lg:px-7">
      <Link
        activeOptions={{ exact: true }}
        activeProps={{ className: 'border-brand text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <MessageSquare className="size-3.5" />
        Discussion
      </Link>
      <Link
        activeProps={{ className: 'border-brand text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId/changes"
      >
        <GitCommit className="size-3.5" />
        Changes
      </Link>
    </nav>
  )
}
