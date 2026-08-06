import type {
  AccountSession,
  RequestDetail,
  RepoLiveState,
  RepoParams,
  RequestMutation,
  RequestRating,
  RequestRatings,
  RequestRevisionCommitFiles,
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type {
  LoadRequestRevisionCommitInput,
  RateRequestInput,
} from '@/api/requests'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import { GitCommit, History, MessageSquare, ShieldQuestion } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import { RequestContextRail } from './request-context-rail'
import {
  RequestChangesWorkbench,
  type RequestChangesSearch,
} from './request-changes-workbench'
import { RequestDescription } from './request-description'
import type {
  CreateDiscussionInput,
  CreateReplyInput,
  LoadDiscussionsInput,
  LoadRepliesInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
  RequestDiscussionRepliesPage,
  UpdateDescriptionInput,
} from './request-discussion-api'
import { RequestDiscussionWorkbench } from './request-discussion-workbench'
import type { RequestDiscussionActions } from './request-discussion-store'
import type {
  RequestActivityPage,
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionReplyMutation,
} from './request-discussion-types'
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
      <PageHeader
        actions={(
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/$owner/$repo/requests">
              Requests
            </Link>
          </Button>
        )}
        badges={<Badge variant="warning">Unavailable</Badge>}
        description="This request does not exist or is unavailable to this account."
        title="Request not found"
      />
      <section className="mt-8 border-t border-border py-8">
        <div className="flex max-w-2xl items-start gap-3 text-sm leading-6 text-muted-foreground">
          <ShieldQuestion className="mt-0.5 size-4 shrink-0" />
          <p>Sign in with an account that has access, or return to the request list.</p>
        </div>
      </section>
    </PageContent>
  )
}

type RequestDetailPageProps = {
  account: AccountSession | null
  activeView: 'changes' | 'discussion'
  createDiscussion: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  createReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  detail: RequestDetail
  discussionPage: RequestDiscussionPage
  live: RepoLiveState
  loadActivity: () => Promise<RequestActivityPage>
  loadRevisionCommit: (
    input: LoadRequestRevisionCommitInput,
  ) => Promise<RequestRevisionCommitFiles>
  loadRevisionDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  loadDiscussionChanges: (input: {
    after: number
    owner: string
    repo: string
    request_id: string
  }) => Promise<RequestDiscussionChanges>
  loadReplies: (input: LoadRepliesInput) => Promise<RequestDiscussionRepliesPage>
  markDiscussionRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  onChangesSearchChange: (search: RequestChangesSearch) => void
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  reopenAndReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  revisions: RequestRevisions | null
  ratings: RequestRatings
  rateRequest: (input: RateRequestInput) => Promise<RequestRating>
  resolveDiscussion: (input: RequestDiscussionActionInput) => Promise<RequestDiscussionMutation>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutation>
  search: RequestChangesSearch & { discussion?: string }
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    account,
    activeView,
    createDiscussion,
    createReply,
    detail,
    discussionPage,
    live,
    loadActivity,
    loadRevisionCommit,
    loadRevisionDiff,
    loadDiscussions,
    loadDiscussionChanges,
    loadReplies,
    markDiscussionRead,
    onChangesSearchChange,
    params,
    performAction,
    reopenAndReply,
    revisions,
    ratings,
    rateRequest,
    resolveDiscussion,
    updateDescription,
    search,
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
  const actor = useMemo(() => ({
    handle: account?.user?.handle ?? 'Anonymous',
    id: account?.user?.id ?? 'anonymous',
  }), [account?.user?.handle, account?.user?.id])
  const discussionParams = useMemo(() => ({
    owner: params.owner,
    repo: params.repo,
    request_id: request.id,
  }), [params.owner, params.repo, request.id])
  const discussionActions: RequestDiscussionActions = useMemo(() => ({
    create: createDiscussion,
    load: loadDiscussions,
    loadChanges: loadDiscussionChanges,
    markRead: markDiscussionRead,
    resolve: resolveDiscussion,
  }), [
    createDiscussion,
    loadDiscussionChanges,
    loadDiscussions,
    markDiscussionRead,
    resolveDiscussion,
  ])
  const threadActions = useMemo(
    () => ({ createReply, loadReplies, reopenAndReply }),
    [createReply, loadReplies, reopenAndReply],
  )
  const isMaintainer = live.repo.access.actor !== 'Public'
  const hasLifecycleActions = request.permissions.can_submit ||
    request.permissions.can_merge ||
    request.permissions.can_close

  const canResolveDiscussion = useCallback(
    (discussion: RequestDiscussion) => !['Closed', 'Merged'].includes(request.state) && (
      isMaintainer ||
      actor.id === discussion.author.id ||
      actor.id === request.author_user_id
    ),
    [actor.id, isMaintainer, request.author_user_id, request.state],
  )

  async function saveDescription(nextDescription: string) {
    try {
      await updateDescription({
        ...discussionParams,
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
      <WorkbenchHeader
        actions={(
          <div className="flex flex-wrap items-center justify-end gap-2">
            <RequestLifecycleActions
              actions={requestActions}
              className="hidden xl:flex"
              request={request}
            />
            <Button asChild className="h-9" size="sm" variant="secondary">
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
        )}
        className="sm:flex-col sm:items-stretch xl:flex-row xl:items-end"
        count={<span className="font-mono">{request.name} / {request.id}</span>}
        description={(
          <div className="grid gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <LifecycleBadge state={live.repo.lifecycle_state} />
              <Badge variant={requestStatusTone(request)}>{requestStatusLabel(request)}</Badge>
              {request.state === 'Open' ? (
                <Badge variant={requestMergeabilityTone(request)}>
                  {requestMergeabilityLabel(request)}
                </Badge>
              ) : null}
            </div>
            {requestActions.error ? (
              <p className="text-sm text-destructive" role="alert">{requestActions.error}</p>
            ) : null}
          </div>
        )}
        eyebrow="Request"
        title={request.title}
      />
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
          <RequestViewTabs
            activeView={activeView}
            params={{ ...params, requestId: request.id }}
          />
          {activeView === 'discussion' ? (
            <RequestDiscussionWorkbench
              actions={discussionActions}
              actor={actor}
              canResolve={canResolveDiscussion}
              focusedDiscussionId={search.discussion}
              initialPage={discussionPage}
              params={discussionParams}
              permissions={{
                canOpenDiscussion: request.permissions.can_open_discussion,
                canReply: request.permissions.can_reply_to_discussion,
              }}
              repoId={live.repo.id}
              request={request}
              threadActions={threadActions}
            />
          ) : revisions ? (
            <RequestChangesWorkbench
              audience={live.repo.access.can_read_private_files ? 'private' : 'public'}
              loadCommit={loadRevisionCommit}
              loadDiff={loadRevisionDiff}
              loadDiscussions={loadDiscussions}
              onSearchChange={onChangesSearchChange}
              params={discussionParams}
              repoId={live.repo.id}
              revisions={revisions}
              search={search}
            />
          ) : (
            <section className="border-b border-border px-5 py-14 text-center lg:px-7">
              <GitCommit className="mx-auto size-5 text-muted-foreground" />
              <h2 className="mt-3 text-sm font-semibold">Changes are unavailable</h2>
              <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
                The request conversation is still available. Refresh to try loading its revision history again.
              </p>
            </section>
          )}
        </div>
        <RequestContextRail
          actions={requestActions}
          onRate={rateRequest}
          params={discussionParams}
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
  activeView,
  params,
}: {
  activeView: 'changes' | 'discussion'
  params: RepoParams & { requestId: string }
}) {
  const tabClass = 'inline-flex h-11 items-center gap-2 border-b-2 px-1 text-sm font-medium transition-colors'
  return (
    <nav aria-label="Request views" className="flex gap-6 border-b border-border px-5 lg:px-7">
      <Link
        aria-current={activeView === 'discussion' ? 'page' : undefined}
        className={`${tabClass} ${activeView === 'discussion' ? 'border-brand text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'}`}
        params={params}
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <MessageSquare className="size-3.5" />
        Discussion
      </Link>
      <Link
        aria-current={activeView === 'changes' ? 'page' : undefined}
        className={`${tabClass} ${activeView === 'changes' ? 'border-brand text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'}`}
        params={params}
        search={{}}
        to="/$owner/$repo/requests/$requestId/changes"
      >
        <GitCommit className="size-3.5" />
        Changes
      </Link>
    </nav>
  )
}
