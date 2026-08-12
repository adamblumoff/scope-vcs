import type {
  CommitDetail,
  CommitFile,
  CommitSummary,
  ProjectionPreviewAudience,
  RequestRevisionCommitFiles,
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type { LoadRequestRevisionCommitInput } from '@/api/requests'
import {
  historyCommitCacheKey,
  historyDiffCacheKey,
  peekHistoryCommitCache,
  peekHistoryDiffCache,
  readHistoryCommitCache,
  readHistoryDiffCache,
  writeHistoryCommitCache,
  writeHistoryDiffCache,
} from '@/features/history/history-resource-cache'
import { HistoryWorkbench } from '@/features/history/history-workbench'
import type {
  CommitDetailState,
  CommitFileDiffState,
} from '@/features/history/history-state'
import { useCachedResource } from '@/lib/use-cached-resource'
import { GitCommit, MessageSquare } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { compactDiscussionSummary } from './request-discussion-model'
import type { LoadDiscussionsInput } from './request-discussion-api'
import {
  discussionsForRequestCommit,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
  requestRevisionCommitId,
} from './request-changes-model'
import type { RequestDiscussion } from './request-discussion-types'

export type RequestChangesSearch = {
  commit?: string
  path?: string
  revision?: string
}

type DiscussionReferenceState = {
  discussions: RequestDiscussion[]
  error: string | null
  loadMore?: () => void
  retry?: () => void
  status: 'failed' | 'loaded' | 'loading'
}

export function RequestChangesWorkbench({
  audience,
  loadCommit,
  loadDiff,
  loadDiscussions,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  audience: ProjectionPreviewAudience
  loadCommit: (
    input: LoadRequestRevisionCommitInput,
  ) => Promise<RequestRevisionCommitFiles>
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<{
    discussions: RequestDiscussion[]
    next_cursor: string | null
  }>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisions
  search: RequestChangesSearch
}) {
  const model = useRequestChangesModel({
    audience,
    loadCommit,
    loadDiff,
    onSearchChange,
    params,
    repoId,
    revisions,
    search,
  })
  const discussionReferences = useRequestDiscussionReferences({
    commitOid: model.selectedCommitOid,
    loadDiscussions,
    params,
    revision: model.selectedRevision,
  })
  const references = useMemo(
    () => discussionsForRequestCommit(
      discussionReferences.discussions,
      model.selectedRevision,
      model.selectedCommitOid,
    ),
    [discussionReferences.discussions, model.selectedCommitOid, model.selectedRevision],
  )
  const commitContext = useMemo(
    () => model.selectedRevision ? (
      <RequestCommitContext
        discussions={references}
        discussionReferences={discussionReferences}
        hasEarlierRevisions={revisions.has_earlier_revisions}
        params={params}
        revision={model.selectedRevision}
      />
    ) : undefined,
    [discussionReferences, model.selectedRevision, params, references, revisions.has_earlier_revisions],
  )
  const emptyDescription = model.commitState.status === 'failed'
    ? model.commitState.error ?? 'Request changes are unavailable.'
    : 'Changes appear here after the request branch is pushed.'
  const emptyTitle = model.commitState.status === 'failed'
    ? 'Request changes unavailable'
    : 'No request changes yet'

  return (
    <HistoryWorkbench
      commitContext={commitContext}
      commitState={model.commitState}
      commits={model.commits}
      diffIdentity={model.diffIdentity}
      emptyDescription={emptyDescription}
      emptyTitle={emptyTitle}
      fileDiffState={model.fileDiffState}
      onCloseDiff={model.closeDiff}
      onRetryCommit={model.retryCommit}
      onRetryDiff={model.retryDiff}
      onSelectCommit={model.selectCommit}
      onSelectFile={model.selectFile}
      selectedCommitId={model.selectedCommitId}
      selectedFilePath={model.selectedFilePath}
    />
  )
}

function useRequestDiscussionReferences({
  commitOid,
  loadDiscussions,
  params,
  revision,
}: {
  commitOid: string | null
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<{
    discussions: RequestDiscussion[]
    next_cursor: string | null
  }>
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisions['revisions'][number] | null
}): DiscussionReferenceState {
  const key = revision && commitOid ? `${revision.id}:${commitOid}` : null
  const loadFirstPage = useCallback(async (signal: AbortSignal) => {
    if (!key || !revision || !commitOid) {
      throw new Error('Select a request commit.')
    }
    signal.throwIfAborted()
    const page = await loadDiscussions({
      ...params,
      commit_oid: commitOid,
      include_revision_anchor: commitOid === revision.commits.at(-1)?.oid,
      revision_id: revision.id,
    })
    signal.throwIfAborted()
    return page
  }, [commitOid, key, loadDiscussions, params, revision])
  const firstPage = useCachedResource({
    fallbackError: 'Discussion references are unavailable.',
    identity: key,
    load: loadFirstPage,
    peek: emptyDiscussionReferenceCache,
    read: emptyDiscussionReferenceCache,
    write: discardDiscussionReferencePage,
  })
  const [additional, setAdditional] = useState<{
    discussions: RequestDiscussion[]
    error: string | null
    key: string | null
    nextCursor: string | null
    status: 'failed' | 'loaded' | 'loading'
  }>({ discussions: [], error: null, key: null, nextCursor: null, status: 'loaded' })
  const additionalMatches = additional.key === key
  const activeAdditional = additionalMatches
    ? additional
    : { discussions: [], error: null, key, nextCursor: null, status: 'loaded' as const }
  const nextCursor = additionalMatches
    ? activeAdditional.nextCursor
    : firstPage.value?.next_cursor ?? null
  const loadNextPage = useCallback(async () => {
    if (!key || !revision || !commitOid || !nextCursor) return
    setAdditional((current) => ({
      discussions: current.key === key ? current.discussions : [],
      error: null,
      key,
      nextCursor,
      status: 'loading',
    }))
    try {
      const page = await loadDiscussions({
        ...params,
        commit_oid: commitOid,
        cursor: nextCursor,
        include_revision_anchor: commitOid === revision.commits.at(-1)?.oid,
        revision_id: revision.id,
      })
      setAdditional((current) => current.key === key ? {
        discussions: [...current.discussions, ...page.discussions],
        error: null,
        key,
        nextCursor: page.next_cursor,
        status: 'loaded',
      } : current)
    } catch (error) {
      setAdditional((current) => current.key === key ? {
        ...current,
        error: error instanceof Error ? error.message : 'Discussion references are unavailable.',
        status: 'failed',
      } : current)
    }
  }, [commitOid, key, loadDiscussions, nextCursor, params, revision])
  const firstDiscussions = firstPage.value?.discussions ?? []
  const status = firstPage.status === 'failed' || activeAdditional.status === 'failed'
    ? 'failed'
    : firstPage.status === 'loading' || activeAdditional.status === 'loading'
      ? 'loading'
      : 'loaded'
  return {
    discussions: [...firstDiscussions, ...activeAdditional.discussions],
    error: firstPage.error ?? activeAdditional.error,
    loadMore: nextCursor && status !== 'loading'
      ? () => void loadNextPage()
      : undefined,
    retry: firstPage.status === 'failed'
      ? firstPage.retry
      : activeAdditional.status === 'failed'
        ? () => void loadNextPage()
      : undefined,
    status,
  }
}

function emptyDiscussionReferenceCache() {
  return null
}

function discardDiscussionReferencePage() {}

function useRequestChangesModel({
  audience,
  loadCommit,
  loadDiff,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  audience: ProjectionPreviewAudience
  loadCommit: (
    input: LoadRequestRevisionCommitInput,
  ) => Promise<RequestRevisionCommitFiles>
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
  ) => Promise<ReviewFileDiff>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisions
  search: RequestChangesSearch
}) {
  const orderedRevisions = revisions.revisions
  const commits = useMemo(
    () => orderedRequestCommits(orderedRevisions),
    [orderedRevisions],
  )
  const selection = useMemo(
    () => requestChangeSelection(
      orderedRevisions,
      revisions.review_revision_id,
      search,
    ),
    [orderedRevisions, revisions.review_revision_id, search],
  )
  const selectedRevision = selection.revision
  const selectedCommitOid = selection.commit
  const selectedCommitId = selectedRevision && selectedCommitOid
    ? requestRevisionCommitId(selectedRevision.id, selectedCommitOid)
    : null
  const generation = `${orderedRevisions.at(-1)?.position ?? 0}`
  const viewKey = `request:${params.request_id}:${selectedRevision?.id ?? 'none'}`
  const commitIdentity = selectedCommitId && selectedRevision
    ? historyCommitCacheKey({
        audience,
        commit: selectedCommitId,
        generation,
        repoId,
        viewKey,
      })
    : null
  const loadSelectedCommit = useCallback(
    async (signal: AbortSignal) => {
      if (!selectedCommitOid || !selectedRevision) {
        throw new Error('Select a request commit.')
      }
      signal.throwIfAborted()
      const result = await loadCommit({
        ...params,
        commit_oid: selectedCommitOid,
        revision_id: selectedRevision.id,
      })
      signal.throwIfAborted()
      return commitDetail(result, audience, repoId, params.request_id)
    },
    [audience, loadCommit, params, repoId, selectedCommitOid, selectedRevision],
  )
  const commitResource = useCachedResource({
    fallbackError: 'Request commit is unavailable.',
    identity: commitIdentity,
    load: loadSelectedCommit,
    peek: peekHistoryCommitCache,
    read: readHistoryCommitCache,
    write: writeHistoryCommitCache,
  })
  const selectedCommit = commitResource.value
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedCommit?.files.find(
    ({ path }) => path === selectedFilePath,
  ) ?? null
  const diffIdentity = selectedCommitId && selectedFile && selectedRevision
    ? historyDiffCacheKey({
        audience,
        commit: selectedCommitId,
        generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId,
        viewKey,
      })
    : null
  const loadSelectedDiff = useCallback(
    async (signal: AbortSignal) => {
      if (!selectedCommitOid || !selectedRevision || !selectedFilePath) {
        throw new Error('Select a changed file.')
      }
      signal.throwIfAborted()
      const result = await loadDiff({
        ...params,
        commit_oid: selectedCommitOid,
        path: selectedFilePath,
        revision_id: selectedRevision.id,
      })
      signal.throwIfAborted()
      return result
    },
    [loadDiff, params, selectedCommitOid, selectedFilePath, selectedRevision],
  )
  const diffResource = useCachedResource({
    fallbackError: 'Request file diff is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    peek: peekHistoryDiffCache,
    read: readHistoryDiffCache,
    write: writeHistoryDiffCache,
  })
  const commitState: CommitDetailState = selection.error
    ? { commit: null, error: selection.error, status: 'failed' }
    : resourceToCommitState(commitResource)
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedCommit && !selectedFile
      ? { diff: null, error: 'This file is not part of the selected commit.', status: 'failed' }
      : resourceToDiffState(diffResource)

  function replaceSelection(
    revision: typeof selectedRevision,
    commitOid: string | null,
    path: string | null,
  ) {
    onSearchChange({
      commit: commitOid ?? undefined,
      path: path ?? undefined,
      revision: revision?.id,
    })
  }

  return {
    closeDiff: () => replaceSelection(selectedRevision, selectedCommitOid, null),
    commits,
    commitState,
    diffIdentity,
    fileDiffState,
    retryCommit: selection.error ? undefined : commitResource.retry,
    retryDiff: selectedFilePath && selectedCommit && !selectedFile
      ? undefined
      : diffResource.retry,
    selectCommit: (commit: CommitSummary) => {
      const selected = requestCommitForListId(orderedRevisions, commit.projected_id)
      replaceSelection(selected?.revision ?? null, selected?.commitOid ?? null, null)
    },
    selectFile: (file: CommitFile) =>
      replaceSelection(selectedRevision, selectedCommitOid, file.path),
    selectedCommitId,
    selectedCommitOid,
    selectedFilePath,
    selectedRevision,
  }
}

function RequestCommitContext({
  discussionReferences,
  discussions,
  hasEarlierRevisions,
  params,
  revision,
}: {
  discussionReferences: DiscussionReferenceState
  discussions: RequestDiscussion[]
  hasEarlierRevisions: boolean
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisions['revisions'][number]
}) {
  return (
    <div className="mt-3 border-t border-border pt-3 text-xs text-muted-foreground">
      <div className="flex flex-wrap items-center gap-2">
        <GitCommit className="size-3.5" />
        <span>Revision {revision.position} by {revision.actor.handle}</span>
        {revision.old_head_oid && revision.new_head_oid ? (
          <span className="font-mono">
            {shortOid(revision.old_head_oid)} → {shortOid(revision.new_head_oid)}
          </span>
        ) : null}
      </div>
      {revision.inspection !== 'Complete' || hasEarlierRevisions ? (
        <p className="mt-2">
          {[
            revision.inspection === 'Incomplete'
              ? 'Commit inspection for this revision is incomplete.'
              : null,
            revision.inspection === 'Unavailable'
              ? 'Commit inspection for this revision is unavailable.'
              : null,
            hasEarlierRevisions ? 'Earlier request revisions are omitted.' : null,
          ].filter(Boolean).join(' ')}
        </p>
      ) : null}
      {discussions.length > 0 ? (
        <div className="mt-3 space-y-2" aria-label="Related discussions">
          {discussions.map((discussion) => (
            <a
              className="flex min-w-0 items-center gap-2 text-foreground hover:text-brand"
              href={discussionHref(params, discussion.id)}
              key={discussion.id}
            >
              <MessageSquare className="size-3.5 shrink-0" />
              <span className="truncate">{compactDiscussionSummary(discussion.body_markdown)}</span>
              <span className="ml-auto shrink-0 text-muted-foreground">
                {discussion.status === 'Resolved' ? 'Resolved' : 'Open'}
              </span>
            </a>
          ))}
        </div>
      ) : null}
      {discussionReferences.status === 'loading' ? (
        <p className="mt-3">Loading discussion references…</p>
      ) : null}
      {discussionReferences.status === 'failed' ? (
        <div className="mt-3 flex items-center gap-2">
          <span>{discussionReferences.error}</span>
          <button
            className="font-medium text-foreground hover:text-brand"
            onClick={discussionReferences.retry}
            type="button"
          >
            Retry
          </button>
        </div>
      ) : null}
      {discussionReferences.loadMore ? (
        <button
          className="mt-3 font-medium text-foreground hover:text-brand"
          onClick={discussionReferences.loadMore}
          type="button"
        >
          Load more discussion references
        </button>
      ) : null}
    </div>
  )
}

function commitDetail(
  result: RequestRevisionCommitFiles,
  audience: ProjectionPreviewAudience,
  repoId: string,
  requestId: string,
): CommitDetail {
  return {
    audience,
    author: result.commit.author,
    change_count: result.files.length,
    files: result.files.map((file) => ({
      ...file,
      path: `/${file.path.replace(/^\/+/, '')}`,
    })),
    logical_commit_id: result.commit.oid,
    message: result.commit.message,
    parent_projected_id: result.commit.parent_oids[0] ?? null,
    projected_id: requestRevisionCommitId(result.revision_id, result.commit.oid),
    repo_id: repoId,
    view_key: `request:${requestId}:${result.revision_id}`,
  }
}

function resourceToCommitState(
  resource: ReturnType<typeof useCachedResource<CommitDetail>>,
): CommitDetailState {
  if (resource.status === 'loaded') {
    return { commit: resource.value, error: null, status: 'loaded' }
  }
  if (resource.status === 'failed') {
    return { commit: null, error: resource.error, status: 'failed' }
  }
  return { commit: null, error: null, status: resource.status }
}

function resourceToDiffState(
  resource: ReturnType<typeof useCachedResource<ReviewFileDiff>>,
): CommitFileDiffState {
  if (resource.status === 'loaded') {
    return { diff: resource.value, error: null, status: 'loaded' }
  }
  if (resource.status === 'failed') {
    return { diff: null, error: resource.error, status: 'failed' }
  }
  return { diff: null, error: null, status: resource.status }
}

function discussionHref(
  params: { owner: string; repo: string; request_id: string },
  discussionId: string,
) {
  const search = new URLSearchParams({ discussion: discussionId })
  return `/${encodeURIComponent(params.owner)}/${encodeURIComponent(params.repo)}/requests/${encodeURIComponent(params.request_id)}?${search}#discussion-${encodeURIComponent(discussionId)}`
}

function shortOid(oid: string) {
  return oid.slice(0, 8)
}
