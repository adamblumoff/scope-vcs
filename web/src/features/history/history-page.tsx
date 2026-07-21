import type {
  CommitDetail,
  CommitFile,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
  RequestChangeBlockFiles,
  ReviewFileDiff,
} from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { RepoShell } from '@/components/repo-shell'
import { RouteErrorPage } from '@/components/route-error-page'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import {
  workspaceTabDomIds,
  workspaceTabPanelId,
  type WorkspaceTabItem,
} from '@/components/workspace-tab-model'
import { WorkspaceTabStrip } from '@/components/workspace-tab-strip'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  historyCommitCacheKey,
  historyDiffCacheKey,
  readHistoryCommitCache,
  readHistoryDiffCache,
  readHistoryDiffScroll,
  writeHistoryCommitCache,
  writeHistoryDiffCache,
  writeHistoryDiffScroll,
} from '@/features/history/history-resource-cache'
import {
  useHistoryResource,
  type HistoryResource,
} from '@/features/history/use-history-resource'
import { cn } from '@/lib/utils'
import {
  loadCommitDetail,
  loadCommitFileDiff,
  loadRequestRevision,
  loadRequestRevisionFileDiff,
} from '@/routes/-repo-history-actions'
import { useNavigate } from '@tanstack/react-router'
import {
  Globe2,
  GitCommit,
  History,
  LockKeyhole,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useCallback, useMemo, useRef } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import { audienceLabel, changeCountLabel } from '../review/review-labels'

const HISTORY_TAB_SET_ID = 'history-file-diffs'

export type CommitHistories = {
  private: CommitHistory | null
  public: CommitHistory | null
}

type HistoryPageProps = {
  histories: CommitHistories
  params: RepoParams
  search: {
    audience?: ProjectionPreviewAudience
    commit?: string
    path?: string
    request?: string
    revision?: string
  }
}

export function HistoryPage(props: HistoryPageProps) {
  const { params } = props
  const {
    audience,
    activateFileTab,
    availableAudiences,
    closeFileTab,
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    fileTabs,
    history,
    repoId,
    requestRevision,
    retryCommit,
    retryDiff,
    selectAudience,
    selectCommit,
    selectFile,
    selectedCommit,
    selectedCommitId,
    selectedFilePath,
  } = useHistoryPageModel(props)

  return (
    <RepoShell params={params}>
      <WorkbenchHeader
        actions={!requestRevision && availableAudiences.length > 1 ? (
          <AudienceToggle
            audience={audience}
            availableAudiences={availableAudiences}
            onSelect={selectAudience}
          />
        ) : undefined}
        count={`${commits.length} ${commits.length === 1 ? 'commit' : 'commits'}${selectedCommit ? ` · ${changeCountLabel(selectedCommit.change_count)}` : ''}`}
        description={requestRevision
          ? `Files for request ${requestRevision.request}.`
          : `Projected commit history for ${repoId}.`}
        eyebrow={requestRevision ? 'Request revision' : `${audienceLabel(audience)} view`}
        title={requestRevision ? 'Revision' : 'History'}
      />
      <section className="px-4 pb-10 sm:px-6 lg:px-8">
        <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <History className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-semibold leading-5">Commits</h2>
            </div>
          </div>
        </div>

        {(!history || commits.length === 0) && !requestRevision ? (
          <div className="flex flex-col items-center gap-3 py-16 text-center">
            <div className="flex size-11 items-center justify-center rounded-xl bg-brand-muted text-brand">
              <History className="size-5" />
            </div>
            <div className="text-sm">
              <div className="text-base font-semibold leading-6">
                No commits yet
              </div>
              <p className="mt-0.5 text-muted-foreground">
                History appears here once Scope has applied commits.
              </p>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-1 lg:grid-cols-[minmax(260px,0.46fr)_minmax(0,1.54fr)]">
            <CommitList
              commits={commits}
              onSelectCommit={selectCommit}
              selectedCommitId={selectedCommitId}
            />
            <CommitDetailPanel
              commitState={commitState}
              diffIdentity={diffIdentity}
              diffScrollTop={readHistoryDiffScroll(diffIdentity)}
              fileDiffState={fileDiffState}
              fileTabs={fileTabs}
              onActivateFileTab={activateFileTab}
              onCloseFileTab={closeFileTab}
              onDiffScroll={(scrollTop) => writeHistoryDiffScroll(diffIdentity, scrollTop)}
              onRetryCommit={retryCommit}
              onRetryDiff={retryDiff}
              onSelectFile={selectFile}
              selectedFilePath={selectedFilePath}
            />
          </div>
        )}
      </section>
    </RepoShell>
  )
}

function useHistoryPageModel({ histories, params, search }: HistoryPageProps) {
  const navigate = useNavigate()
  const availableAudiences = useMemo(
    () =>
      (['private', 'public'] as const).filter(
        (option) => histories[option] !== null,
      ),
    [histories],
  )
  const audience = selectedAudience(histories, search.audience)
  const history = histories[audience] ?? histories.public ?? histories.private
  const baseCommits = useMemo(
    () => [...(history?.commits ?? [])].reverse(),
    [history?.commits],
  )
  const requestRevision = useMemo(
    () => search.request && search.revision
      ? { request: search.request, revision: search.revision }
      : null,
    [search.request, search.revision],
  )
  const requestedCommitUnavailable = Boolean(
    !requestRevision && search.commit && history && !history.commits.some(
      (commit) => commit.projected_id === search.commit,
    ),
  )
  const selectedCommitId = requestedCommitUnavailable
    ? null
    : requestRevision?.revision ?? search.commit ?? latestCommitId(history)
  const repoId = history?.repo_id ?? `${params.owner}/${params.repo}`
  const commitIdentity = selectedCommitId && history
    ? historyCommitCacheKey({
        audience,
        commit: selectedCommitId,
        generation: requestRevision?.revision ?? history.generation,
        repoId: history.repo_id,
        viewKey: requestRevision ? `request:${requestRevision.request}` : history.view_key,
      })
    : null
  const loadSelectedCommit = useCallback(
    async (signal: AbortSignal) => {
      if (requestRevision) {
        const result = await loadRequestRevision({
          data: { ...params, ...requestRevision },
          signal,
        })
        return revisionCommitDetail(result, audience, repoId, requestRevision.request)
      }
      return loadCommitDetail({
        data: {
          audience,
          commit: selectedCommitId ?? '',
          owner: params.owner,
          repo: params.repo,
        },
        signal,
      })
    },
    [audience, params, repoId, requestRevision, selectedCommitId],
  )
  const commitResource = useHistoryResource({
    identity: commitIdentity,
    load: loadSelectedCommit,
    read: readHistoryCommitCache,
    write: writeHistoryCommitCache,
  })
  const selectedCommit = commitResource.value
  const commits = useMemo(() => {
    if (!requestRevision || !selectedCommit) return baseCommits
    return [selectedCommit, ...baseCommits.filter(
      (commit) => commit.projected_id !== selectedCommit.projected_id,
    )]
  }, [baseCommits, requestRevision, selectedCommit])
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedCommit?.files.find(
    (file) => file.path === selectedFilePath,
  ) ?? null
  const diffIdentity = selectedCommitId && selectedFile && history
    ? historyDiffCacheKey({
        audience,
        commit: selectedCommitId,
        generation: requestRevision?.revision ?? history.generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId,
        viewKey: requestRevision ? `request:${requestRevision.request}` : history.view_key,
      })
    : null
  const loadSelectedDiff = useCallback(
    (signal: AbortSignal) => requestRevision
      ? loadRequestRevisionFileDiff({
          data: {
            ...params,
            ...requestRevision,
            path: selectedFilePath ?? '',
          },
          signal,
        })
      : loadCommitFileDiff({
          data: {
            audience,
            commit: selectedCommitId ?? '',
            owner: params.owner,
            path: selectedFilePath ?? '',
            repo: params.repo,
          },
          signal,
        }),
    [audience, params, requestRevision, selectedCommitId, selectedFilePath],
  )
  const diffResource = useHistoryResource({
    identity: diffIdentity,
    load: loadSelectedDiff,
    read: readHistoryDiffCache,
    write: writeHistoryDiffCache,
  })
  const commitState: CommitDetailState = requestedCommitUnavailable
    ? { commit: null, error: 'The requested commit is not available in this history view.', status: 'failed' }
    : resourceToCommitState(commitResource)
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedCommit && !selectedFile
      ? { diff: null, error: 'This file is not part of the selected commit.', status: 'failed' }
      : resourceToDiffState(diffResource)
  const fileTabItems = useMemo(
    () =>
      (selectedCommit?.files ?? []).map((file) => ({
        id: file.path,
        label: fileName(file.path),
        title: file.path.replace(/^\/+/, ''),
      })),
    [selectedCommit?.files],
  )
  const fileTabs = useWorkspaceTabs({
    activeId: selectedFilePath,
    items: fileTabItems,
    storageKey: `history:${repoId}:${audience}:${selectedCommitId ?? 'none'}`,
  })

  function replaceHistorySearch(
    nextAudience: ProjectionPreviewAudience,
    nextCommitId: string | null,
    nextPath: string | null = null,
    keepRevision = true,
  ) {
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: {
        audience: nextAudience,
        commit: nextCommitId ?? undefined,
        path: nextPath ?? undefined,
        request: keepRevision ? requestRevision?.request : undefined,
        revision: keepRevision ? requestRevision?.revision : undefined,
      },
      to: '/repos/$owner/$repo/history',
    })
  }

  return {
    activateFileTab: (path: string) =>
      replaceHistorySearch(audience, selectedCommitId, path),
    audience,
    availableAudiences,
    closeFileTab: (path: string) => {
      if (!fileTabs.tabs.some((tab) => tab.id === path)) {
        replaceHistorySearch(audience, selectedCommitId)
        return null
      }
      const result = fileTabs.close(path)
      if (path === selectedFilePath) {
        replaceHistorySearch(audience, selectedCommitId, result.activeId)
      }
      return result.focusId
    },
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    fileTabs: fileTabs.tabs,
    history,
    repoId,
    requestRevision,
    retryCommit: requestedCommitUnavailable ? undefined : commitResource.retry,
    retryDiff: selectedFilePath && selectedCommit && !selectedFile
      ? undefined
      : diffResource.retry,
    selectAudience: (nextAudience: ProjectionPreviewAudience) => {
      const nextHistory = histories[nextAudience]
      if (nextHistory) {
        replaceHistorySearch(nextAudience, latestCommitId(nextHistory))
      }
    },
    selectCommit: (commit: CommitSummary) =>
      replaceHistorySearch(
        audience,
        commit.projected_id,
        null,
        commit.projected_id === requestRevision?.revision,
      ),
    selectFile: (file: CommitFile) => {
      fileTabs.prepareOpen(file.path)
      replaceHistorySearch(audience, selectedCommitId, file.path)
    },
    selectedCommit,
    selectedCommitId,
    selectedFilePath,
  }
}

function CommitList({
  commits,
  onSelectCommit,
  selectedCommitId,
}: {
  commits: CommitSummary[]
  onSelectCommit: (commit: CommitSummary) => void
  selectedCommitId: string | null
}) {
  return (
    <div className="border-b border-border lg:border-b-0 lg:border-r">
      <div className="hidden grid-cols-[minmax(0,1fr)_80px] gap-3 border-b border-border px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid">
        <div>Commit</div>
        <div className="text-right">Files</div>
      </div>
      <div className="divide-y divide-border">
        {commits.map((commit) => {
          const selected = selectedCommitId === commit.projected_id
          return (
            <button
              className={cn(
                'grid w-full grid-cols-[minmax(0,1fr)_80px] gap-x-3 px-2 py-3 text-left text-sm transition-colors hover:bg-muted/70',
                selected &&
                  'bg-brand-muted shadow-[inset_2px_0_0_0_var(--brand)] hover:bg-brand-muted',
              )}
              key={commit.projected_id}
              onClick={() => onSelectCommit(commit)}
              type="button"
            >
              <div className="flex min-w-0 items-center gap-2">
                <GitCommit className="size-4 shrink-0 text-muted-foreground" />
                <span className="truncate font-mono text-xs font-medium">
                  {commitTitle(commit)}
                </span>
              </div>
              <div className="self-center text-right font-mono text-xs text-muted-foreground">
                {commit.change_count}
              </div>
            </button>
          )
        })}
      </div>
    </div>
  )
}

function CommitDetailPanel({
  commitState,
  diffIdentity,
  diffScrollTop,
  fileDiffState,
  fileTabs,
  onActivateFileTab,
  onCloseFileTab,
  onDiffScroll,
  onRetryCommit,
  onRetryDiff,
  onSelectFile,
  selectedFilePath,
}: {
  commitState: CommitDetailState
  diffIdentity: string | null
  diffScrollTop: number
  fileDiffState: CommitFileDiffState
  fileTabs: WorkspaceTabItem[]
  onActivateFileTab: (path: string) => void
  onCloseFileTab: (path: string) => string | null
  onDiffScroll: (scrollTop: number) => void
  onRetryCommit?: () => void
  onRetryDiff?: () => void
  onSelectFile: (file: CommitFile) => void
  selectedFilePath: string | null
}) {
  const fileNavigatorRef = useRef<HTMLDivElement>(null)

  if (commitState.status === 'loading') {
    return <CommitDetailSkeleton />
  }

  if (commitState.status === 'failed') {
    return (
      <PanelState tone="error">
        <TriangleAlert className="size-4 text-destructive" />
        <span>{commitState.error}</span>
        {onRetryCommit && (
          <Button onClick={onRetryCommit} size="sm" type="button" variant="secondary">
            Retry
          </Button>
        )}
      </PanelState>
    )
  }

  if (!commitState.commit) {
    return (
      <PanelState>
        <GitCommit className="size-4 text-muted-foreground" />
        <span>Select a commit</span>
      </PanelState>
    )
  }

  const commit = commitState.commit
  const diffOpen = selectedFilePath !== null
  const activeTabDomIds = selectedFilePath && fileTabs.some((tab) => tab.id === selectedFilePath)
    ? workspaceTabDomIds(HISTORY_TAB_SET_ID, selectedFilePath)
    : null
  function closeUnavailableDiff() {
    if (!selectedFilePath) return
    onCloseFileTab(selectedFilePath)
    requestAnimationFrame(() => fileNavigatorRef.current?.focus())
  }

  return (
    <div className="min-w-0">
      <div className="border-b border-border px-2 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="neutral">{commit.logical_commit_id}</Badge>
          {commit.author && <Badge variant="neutral">{commit.author}</Badge>}
        </div>
        <h3 className="mt-2 truncate font-mono text-sm font-semibold leading-5">
          {commitTitle(commit)}
        </h3>
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]">
        <div
          aria-label="Commit file navigator"
          className="min-w-0 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          ref={fileNavigatorRef}
          tabIndex={-1}
        >
          {commit.files.length === 0 ? (
            <div className="px-2 py-10 text-sm text-muted-foreground">
              No file changes in this commit.
            </div>
          ) : (
            <FileSystemTree
              compactVisibility
              files={commit.files}
              getFileMeta={commitFileStatus}
              metaColumnLabel="Change"
              onSelectFile={onSelectFile}
              selectedFilePath={selectedFilePath}
            />
          )}
        </div>
        <div className="h-[70vh] min-h-[340px] max-h-[720px] min-w-0 overflow-hidden border-border xl:border-l">
          <div className="flex h-full min-h-[340px] min-w-0 flex-col">
            <WorkspaceTabStrip
              activeId={selectedFilePath}
              ariaLabel="Open history diffs"
              onActivate={onActivateFileTab}
              onClose={onCloseFileTab}
              onEmptyFocus={() => fileNavigatorRef.current?.focus()}
              tabSetId={HISTORY_TAB_SET_ID}
              tabs={fileTabs}
            />
            <div
              aria-label={fileTabs.length > 0 && !activeTabDomIds ? 'History diff viewer' : undefined}
              aria-labelledby={activeTabDomIds?.tabId}
              className="min-h-0 flex-1 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              id={workspaceTabPanelId(HISTORY_TAB_SET_ID)}
              role={fileTabs.length > 0 ? 'tabpanel' : undefined}
              tabIndex={fileTabs.length > 0 ? 0 : undefined}
            >
              {diffOpen ? (
                <ReviewFileDiffDrawer
                  cacheKey={diffIdentity}
                  className="min-h-0"
                  diff={fileDiffState.diff}
                  error={fileDiffState.error}
                  loading={fileDiffState.status === 'loading'}
                  onClose={fileTabs.length === 0 ? closeUnavailableDiff : undefined}
                  onRetry={fileDiffState.status === 'failed' ? onRetryDiff : undefined}
                  onScrollTopChange={onDiffScroll}
                  scrollTop={diffScrollTop}
                  selectedPath={selectedFilePath}
                  showHeader={fileTabs.length === 0}
                />
              ) : (
                <PanelState>
                  <span>Select a changed file</span>
                </PanelState>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

function CommitDetailSkeleton() {
  return (
    <div className="min-w-0">
      <div className="border-b border-border px-2 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <Skeleton className="h-5 w-16" />
          <Skeleton className="h-5 w-24" />
        </div>
        <Skeleton className="mt-2 h-4 w-2/3" />
      </div>
      <div className="space-y-3 px-2 py-4">
        {Array.from({ length: 5 }).map((_, index) => (
          <div className="flex items-center gap-2" key={index}>
            <Skeleton className="size-4 rounded" />
            <Skeleton className="h-3.5 w-1/2" />
          </div>
        ))}
      </div>
    </div>
  )
}

function AudienceToggle({
  audience,
  availableAudiences,
  onSelect,
}: {
  audience: ProjectionPreviewAudience
  availableAudiences: ProjectionPreviewAudience[]
  onSelect: (audience: ProjectionPreviewAudience) => void
}) {
  return (
    <ToggleGroup
      onValueChange={(value) => {
        if (value) {
          onSelect(value as ProjectionPreviewAudience)
        }
      }}
      type="single"
      value={audience}
    >
      {(['private', 'public'] as const).map((option) => {
        const Icon = option === 'private' ? LockKeyhole : Globe2
        return (
          <ToggleGroupItem
            aria-label={`${audienceLabel(option)} view`}
            disabled={!availableAudiences.includes(option)}
            key={option}
            value={option}
          >
            <Icon className="size-3" />
            <span>{audienceLabel(option)} view</span>
          </ToggleGroupItem>
        )
      })}
    </ToggleGroup>
  )
}

function PanelState({
  children,
  tone = 'muted',
}: {
  children: ReactNode
  tone?: 'error' | 'muted'
}) {
  return (
    <div
      className={cn(
        'flex min-h-[360px] items-center justify-center gap-2 px-4 text-sm leading-5',
        tone === 'error' ? 'text-destructive' : 'text-muted-foreground',
      )}
    >
      {children}
    </div>
  )
}

type CommitDetailState =
  | { commit: null; error: null; status: 'idle' }
  | { commit: null; error: null; status: 'loading' }
  | { commit: CommitDetail; error: null; status: 'loaded' }
  | { commit: null; error: string; status: 'failed' }

type CommitFileDiffState =
  | { diff: null; error: null; status: 'idle' }
  | { diff: null; error: null; status: 'loading' }
  | { diff: ReviewFileDiff; error: null; status: 'loaded' }
  | { diff: null; error: string; status: 'failed' }

function resourceToCommitState(
  resource: HistoryResource<CommitDetail>,
): CommitDetailState {
  switch (resource.status) {
    case 'idle':
      return { commit: null, error: null, status: 'idle' }
    case 'loading':
      return { commit: null, error: null, status: 'loading' }
    case 'loaded':
      return { commit: resource.value, error: null, status: 'loaded' }
    case 'failed':
      return { commit: null, error: resource.error, status: 'failed' }
  }
}

function resourceToDiffState(
  resource: HistoryResource<ReviewFileDiff>,
): CommitFileDiffState {
  switch (resource.status) {
    case 'idle':
      return { diff: null, error: null, status: 'idle' }
    case 'loading':
      return { diff: null, error: null, status: 'loading' }
    case 'loaded':
      return { diff: resource.value, error: null, status: 'loaded' }
    case 'failed':
      return { diff: null, error: resource.error, status: 'failed' }
  }
}

function selectedAudience(
  histories: CommitHistories,
  requestedAudience?: ProjectionPreviewAudience,
): ProjectionPreviewAudience {
  if (requestedAudience && histories[requestedAudience]) {
    return requestedAudience
  }
  return histories.private ? 'private' : 'public'
}

function latestCommitId(history: CommitHistory | null) {
  return history?.commits.at(-1)?.projected_id ?? null
}

function commitTitle(commit: Pick<CommitSummary, 'message'>) {
  return commit.message.split(/\r?\n/, 1)[0]?.trim() || '(no message)'
}

function fileName(path: string) {
  const displayPath = path.replace(/^\/+/, '')
  return displayPath.split('/').at(-1) ?? displayPath
}

function commitFileStatus(file: CommitFile) {
  return <Badge variant="neutral">{file.kind}</Badge>
}

function revisionCommitDetail(
  result: RequestChangeBlockFiles,
  audience: ProjectionPreviewAudience,
  repoId: string,
  requestId: string,
): CommitDetail {
  const block = result.change_block
  return {
    audience,
    author: null,
    change_count: result.files.length,
    files: result.files.map((file) => ({
      ...file,
      path: `/${file.path.replace(/^\/+/, '')}`,
    })),
    logical_commit_id: block.new_head_oid.slice(0, 12),
    message: 'Request update',
    parent_projected_id: block.old_head_oid,
    projected_id: block.id,
    repo_id: repoId,
    view_key: `request:${requestId}`,
  }
}

export function HistoryError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected history error"
      title="History unavailable"
    />
  )
}
