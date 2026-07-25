import type {
  AccountSession,
  RequestChangeBlockFiles,
  RequestDetail,
  RepoLiveState,
  RepoParams,
  RequestMutation,
} from '@/api/types'
import type { LoadRequestChangeBlockFilesInput } from '@/api/requests'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import { Coins, History, ShieldQuestion } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useMemo, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import { RequestContextRail } from './request-context-rail'
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
    <RepoShell params={params}>
      <PageContent>
        <PageHeader
          actions={(
            <Button asChild size="sm" variant="secondary">
              <Link params={params} to="/repos/$owner/$repo/requests">
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
    </RepoShell>
  )
}

type RequestDetailPageProps = {
  account: AccountSession | null
  createDiscussion: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  createReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  detail: RequestDetail
  discussionPage: RequestDiscussionPage
  live: RepoLiveState
  loadActivity: () => Promise<RequestActivityPage>
  loadChangeBlockFiles: (input: LoadRequestChangeBlockFilesInput) => Promise<RequestChangeBlockFiles>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  loadDiscussionChanges: (input: {
    after: number
    owner: string
    repo: string
    request_id: string
  }) => Promise<RequestDiscussionChanges>
  loadReplies: (input: LoadRepliesInput) => Promise<RequestDiscussionRepliesPage>
  markDiscussionRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  reopenAndReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  reopenDiscussion: (input: RequestDiscussionActionInput) => Promise<RequestDiscussionMutation>
  resolveDiscussion: (input: RequestDiscussionActionInput) => Promise<RequestDiscussionMutation>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutation>
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    account,
    createDiscussion,
    createReply,
    detail,
    discussionPage,
    live,
    loadActivity,
    loadChangeBlockFiles,
    loadDiscussions,
    loadDiscussionChanges,
    loadReplies,
    markDiscussionRead,
    params,
    performAction,
    reopenAndReply,
    reopenDiscussion,
    resolveDiscussion,
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
    reopen: reopenDiscussion,
    resolve: resolveDiscussion,
  }), [
    createDiscussion,
    loadDiscussionChanges,
    loadDiscussions,
    markDiscussionRead,
    reopenDiscussion,
    resolveDiscussion,
  ])
  const threadActions = useMemo(
    () => ({ createReply, loadReplies, reopenAndReply }),
    [createReply, loadReplies, reopenAndReply],
  )
  const isMaintainer = live.repo.access.actor !== 'Public'
  const isContributor = actor.id === request.author_user_id ||
    request.invitees.some((invitee) => invitee.user.id === actor.id)
  const heldContributor = request.held_at_unix !== null && isContributor && !isMaintainer
  const hasLifecycleActions = request.permissions.can_mark_ready ||
    request.permissions.can_return_to_working ||
    request.permissions.can_hold ||
    request.permissions.can_request_changes ||
    request.permissions.can_assess ||
    request.permissions.can_merge ||
    request.permissions.can_close

  const canResolveDiscussion = useCallback(
    (discussion: RequestDiscussion) => request.state !== 'Completed' && (
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

  function requestHeader(discussionControls?: ReactNode) {
    return (
      <WorkbenchHeader
        actions={(
          <div className="flex flex-wrap items-center justify-end gap-2">
            {discussionControls}
            <RequestLifecycleActions
              actions={requestActions}
              balance={account?.credit_balance_credits ?? null}
              className="hidden xl:flex"
              request={request}
            />
            <Button asChild className="h-9" size="sm" variant="secondary">
              <Link params={params} to="/repos/$owner/$repo/requests">Requests</Link>
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
              {request.held_at_unix !== null ? <Badge variant="warning">On hold</Badge> : null}
              <Badge variant={requestMergeabilityTone(request)}>{requestMergeabilityLabel(request)}</Badge>
              {request.current_stake_credits > 0 ? (
                <Badge variant="neutral"><Coins />{request.current_stake_credits}</Badge>
              ) : null}
            </div>
            {heldContributor ? (
              <p className="text-sm text-warning-foreground">
                Maintainer review is on hold. Author and invitee changes are disabled until release.
              </p>
            ) : null}
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
    <RepoShell contentClassName={hasLifecycleActions ? 'pb-20 xl:pb-0' : undefined} params={params}>
      <RequestDiscussionWorkbench
        actions={discussionActions}
        actor={actor}
        canResolve={canResolveDiscussion}
        contextRail={<RequestContextRail actions={requestActions} request={request} />}
        description={description}
        header={requestHeader}
        initialPage={discussionPage}
        loadChangeBlockFiles={loadChangeBlockFiles}
        onDescriptionSave={saveDescription}
        params={discussionParams}
        permissions={{
          canEditDescription: request.permissions.can_edit_identity,
          canOpenDiscussion: request.permissions.can_open_discussion,
          canReply: request.permissions.can_reply_to_discussion,
        }}
        repoId={live.repo.id}
        request={request}
        threadActions={threadActions}
      />

      {hasLifecycleActions ? (
        <div className="fixed inset-x-0 bottom-0 z-30 border-t border-[var(--border-strong)] bg-background/95 px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur xl:hidden">
          <RequestLifecycleActions
            actions={requestActions}
            balance={account?.credit_balance_credits ?? null}
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
    </RepoShell>
  )
}
