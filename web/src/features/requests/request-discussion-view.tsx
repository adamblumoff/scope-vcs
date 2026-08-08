import type { AccountSession, RequestDetail, RepoLiveState, RepoParams } from '@/api/types'
import { useCallback, useMemo } from 'react'
import type {
  CreateDiscussionInput,
  CreateReplyInput,
  LoadDiscussionsInput,
  LoadRepliesInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import type { RequestDiscussionActions } from './request-discussion-store'
import type {
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionReplyMutation,
} from './request-discussion-types'
import { RequestDiscussionWorkbench } from './request-discussion-workbench'

type RequestDiscussionViewProps = {
  account: AccountSession | null
  createDiscussion: (input: CreateDiscussionInput) => Promise<RequestDiscussionMutation>
  createReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  detail: RequestDetail
  focusedDiscussionId?: string
  initialPage: RequestDiscussionPage
  live: RepoLiveState
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
  reopenAndReply: (input: CreateReplyInput) => Promise<RequestDiscussionReplyMutation>
  resolveDiscussion: (input: RequestDiscussionActionInput) => Promise<RequestDiscussionMutation>
}

export function RequestDiscussionView({
  account,
  createDiscussion,
  createReply,
  detail,
  focusedDiscussionId,
  initialPage,
  live,
  loadDiscussions,
  loadDiscussionChanges,
  loadReplies,
  markDiscussionRead,
  params,
  reopenAndReply,
  resolveDiscussion,
}: RequestDiscussionViewProps) {
  const { request } = detail
  const actor = useMemo(() => ({
    handle: account?.user?.handle ?? 'Anonymous',
    id: account?.user?.id ?? 'anonymous',
  }), [account?.user?.handle, account?.user?.id])
  const requestParams = useMemo(() => ({
    owner: params.owner,
    repo: params.repo,
    request_id: request.id,
  }), [params.owner, params.repo, request.id])
  const actions: RequestDiscussionActions = useMemo(() => ({
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
  const canResolve = useCallback(
    (discussion: RequestDiscussion) => !['Closed', 'Merged'].includes(request.state) && (
      isMaintainer ||
      actor.id === discussion.author.id ||
      actor.id === request.author_user_id
    ),
    [actor.id, isMaintainer, request.author_user_id, request.state],
  )

  return (
    <RequestDiscussionWorkbench
      actions={actions}
      actor={actor}
      canResolve={canResolve}
      focusedDiscussionId={focusedDiscussionId}
      initialPage={initialPage}
      params={requestParams}
      permissions={{
        canOpenDiscussion: request.permissions.can_open_discussion,
        canReply: request.permissions.can_reply_to_discussion,
      }}
      repoId={live.repo.id}
      request={request}
      threadActions={threadActions}
    />
  )
}
