import type {
  CommitDetail,
  CommitFile,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
  ReviewFileDiff,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { PageContent, PageHeader } from '@/components/page-header'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'
import {
  loadCommitDetail,
  loadCommitFileDiff,
} from '@/routes/-repo-history-actions'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  Globe2,
  GitCommit,
  History,
  TriangleAlert,
  UserRound,
} from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useReducer, useRef } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import { ReviewTree } from '../review/review-tree'
import { audienceLabel, changeCountLabel } from '../review/review-labels'

export type CommitHistories = {
  owner: CommitHistory | null
  public: CommitHistory | null
}

type HistoryPageProps = {
  histories: CommitHistories
  initialAudience: ProjectionPreviewAudience
  initialCommit: CommitDetail | null
  params: RepoParams
}

export function HistoryPage(props: HistoryPageProps) {
  const { params } = props
  const {
    audience,
    availableAudiences,
    closeFileDiff,
    commitState,
    commits,
    fileDiffState,
    history,
    pageWidthClassName,
    repoId,
    selectAudience,
    selectCommit,
    selectFile,
    selectedCommit,
    selectedCommitId,
    selectedFilePath,
  } = useHistoryPageModel(props)

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        breadcrumb={() => <RepoBreadcrumb params={params} section="history" />}
        contentClassName={pageWidthClassName}
      />

      <PageContent className={pageWidthClassName}>
        <PageHeader
          badges={() => (
            <>
              <Badge variant="info">{audienceLabel(audience)} view</Badge>
              <Badge variant="neutral">
                {commits.length} {commits.length === 1 ? 'commit' : 'commits'}
              </Badge>
              {selectedCommit && (
                <Badge variant="neutral">
                  {changeCountLabel(selectedCommit.change_count)}
                </Badge>
              )}
            </>
          )}
          title={
            <>
              <Link
                className="underline decoration-muted-foreground/50 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-4 focus-visible:ring-offset-background"
                params={params}
                to="/repos/$owner/$repo"
              >
                {repoId}
              </Link>{' '}
              history
            </>
          }
          titleClassName="font-mono"
        />

        <section className="mt-8">
          <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <History className="size-4 text-muted-foreground" />
                <h2 className="text-sm font-semibold leading-5">Commits</h2>
              </div>
            </div>
            {availableAudiences.length > 1 && (
              <AudienceToggle
                audience={audience}
                availableAudiences={availableAudiences}
                onSelect={selectAudience}
              />
            )}
          </div>

          {!history || commits.length === 0 ? (
            <div className="flex flex-col items-center gap-3 py-16 text-center">
              <div className="flex size-11 items-center justify-center rounded-xl bg-brand-muted text-brand">
                <History className="size-5" />
              </div>
              <div className="text-sm">
                <div className="text-base font-semibold leading-6">
                  No commits yet
                </div>
                <p className="mt-0.5 text-muted-foreground">
                  History appears here once this repo has published commits.
                </p>
              </div>
            </div>
          ) : (
            <div
              className={cn(
                'grid grid-cols-1 transition-[grid-template-columns] duration-300 ease-out lg:grid-cols-[minmax(260px,0.62fr)_minmax(0,1.38fr)]',
                selectedFilePath &&
                  'xl:grid-cols-[minmax(260px,0.46fr)_minmax(0,1.54fr)]',
              )}
            >
              <CommitList
                commits={commits}
                onSelectCommit={selectCommit}
                selectedCommitId={selectedCommitId}
              />
              <CommitDetailPanel
                commitState={commitState}
                fileDiffState={fileDiffState}
                onCloseFileDiff={closeFileDiff}
                onSelectFile={selectFile}
                selectedFilePath={selectedFilePath}
              />
            </div>
          )}
        </section>
      </PageContent>
    </main>
  )
}

function useHistoryPageModel({
  histories,
  initialAudience,
  initialCommit,
  params,
}: HistoryPageProps) {
  const navigate = useNavigate()
  const [state, dispatch] = useReducer(
    historyPageReducer,
    { histories, initialAudience, initialCommit },
    createHistoryPageState,
  )
  const {
    audience,
    commitState,
    fileDiffState,
    selectedCommitId,
    selectedFilePath,
  } = state
  const availableAudiences = useMemo(
    () =>
      (['owner', 'public'] as const).filter(
        (option) => histories[option] !== null,
      ),
    [histories],
  )
  const history = histories[audience] ?? histories.public ?? histories.owner
  const commits = useMemo(
    () => [...(history?.commits ?? [])].reverse(),
    [history?.commits],
  )
  const selectedCommit =
    commitState.status === 'loaded' ? commitState.commit : null
  const pageWidthClassName = selectedFilePath
    ? 'max-w-[1320px] transition-[max-width] duration-300 ease-out'
    : 'max-w-[1040px] transition-[max-width] duration-300 ease-out'
  const repoId = `${params.owner}/${params.repo}`
  const initialSelectedCommitId =
    initialCommit?.projected_id ??
    latestCommitId(histories[initialAudience]) ??
    null
  const initialCommitKey = initialCommit
    ? commitRequestKey(initialCommit.audience, initialCommit.projected_id)
    : null
  const commitRequestKeyValue =
    selectedCommitId && history
      ? commitRequestKey(audience, selectedCommitId)
      : null
  const commitRequestAudience = commitRequestKeyValue ? audience : null
  const commitRequestProjectedId = commitRequestKeyValue
    ? selectedCommitId
    : null
  const historySourceRef = useRef({
    histories,
    initialAudience,
    initialCommit,
  })
  const activeCommitKeyRef = useRef<string | null>(commitRequestKeyValue)
  const selectedFilePathInCommit =
    selectedCommit &&
    selectedFilePath &&
    selectedCommit.files.some((file) => file.path === selectedFilePath)
      ? selectedFilePath
      : null
  const diffRequestKeyValue =
    selectedCommit && selectedFilePathInCommit
      ? fileDiffRequestKey(
          audience,
          selectedCommit.projected_id,
          selectedFilePathInCommit,
        )
      : null
  const diffRequestProjectedId =
    selectedCommit && selectedFilePathInCommit
      ? selectedCommit.projected_id
      : null
  const diffRequestPath = selectedFilePathInCommit
  const activeDiffKeyRef = useRef<string | null>(diffRequestKeyValue)
  const historySource = historySourceRef.current

  if (
    historySource.histories !== histories ||
    historySource.initialAudience !== initialAudience ||
    historySource.initialCommit !== initialCommit
  ) {
    historySourceRef.current = { histories, initialAudience, initialCommit }
    activeCommitKeyRef.current = initialSelectedCommitId
      ? commitRequestKey(initialAudience, initialSelectedCommitId)
      : null
    activeDiffKeyRef.current = null
    dispatch({
      state: createHistoryPageState({
        histories,
        initialAudience,
        initialCommit,
      }),
      type: 'stateReset',
    })
  } else if (activeCommitKeyRef.current !== commitRequestKeyValue) {
    activeCommitKeyRef.current = commitRequestKeyValue
    activeDiffKeyRef.current = null
    dispatch({
      commit:
        commitRequestKeyValue === initialCommitKey ? initialCommit : null,
      loading:
        commitRequestKeyValue !== null &&
        commitRequestKeyValue !== initialCommitKey,
      type: 'commitRequestChanged',
    })
  } else if (activeDiffKeyRef.current !== diffRequestKeyValue) {
    activeDiffKeyRef.current = diffRequestKeyValue
    dispatch({
      loading: diffRequestKeyValue !== null,
      type: 'fileDiffRequestChanged',
    })
  }

  useEffect(() => {
    if (
      !commitRequestKeyValue ||
      commitRequestAudience === null ||
      commitRequestProjectedId === null
    ) {
      return
    }

    if (commitRequestKeyValue === initialCommitKey) {
      return
    }

    let active = true
    loadCommitDetail({
      data: {
        audience: commitRequestAudience,
        commit: commitRequestProjectedId,
        owner: params.owner,
        repo: params.repo,
      },
    }).then(
      (commit) => {
        if (active) {
          dispatch({ commit, type: 'commitLoaded' })
        }
      },
      (error) => {
        if (active) {
          dispatch({
            message:
              error instanceof Error ? error.message : 'commit load failed',
            type: 'commitFailed',
          })
        }
      },
    )

    return () => {
      active = false
    }
  }, [
    commitRequestAudience,
    commitRequestKeyValue,
    commitRequestProjectedId,
    initialCommitKey,
    params.owner,
    params.repo,
  ])

  useEffect(() => {
    if (
      !diffRequestKeyValue ||
      diffRequestPath === null ||
      diffRequestProjectedId === null
    ) {
      return
    }

    let active = true
    loadCommitFileDiff({
      data: {
        audience,
        commit: diffRequestProjectedId,
        owner: params.owner,
        path: diffRequestPath,
        repo: params.repo,
      },
    }).then(
      (diff) => {
        if (active) {
          dispatch({ diff, type: 'fileDiffLoaded' })
        }
      },
      (error) => {
        if (active) {
          dispatch({
            message: error instanceof Error ? error.message : 'diff load failed',
            type: 'fileDiffFailed',
          })
        }
      },
    )

    return () => {
      active = false
    }
  }, [
    audience,
    diffRequestKeyValue,
    diffRequestPath,
    diffRequestProjectedId,
    params.owner,
    params.repo,
  ])

  function selectAudience(nextAudience: ProjectionPreviewAudience) {
    const nextHistory = histories[nextAudience]
    if (!nextHistory) {
      return
    }

    const nextCommitId = latestCommitId(nextHistory)
    dispatch({
      audience: nextAudience,
      commitId: nextCommitId,
      type: 'audienceSelected',
    })
    replaceHistorySearch(nextAudience, nextCommitId)
  }

  function selectCommit(commit: CommitSummary) {
    dispatch({ commitId: commit.projected_id, type: 'commitSelected' })
    replaceHistorySearch(audience, commit.projected_id)
  }

  function replaceHistorySearch(
    nextAudience: ProjectionPreviewAudience,
    nextCommitId: string | null,
  ) {
    void navigate({
      params,
      replace: true,
      search: {
        audience: nextAudience,
        commit: nextCommitId ?? undefined,
      },
      to: '/repos/$owner/$repo/history',
    })
  }

  return {
    audience,
    availableAudiences,
    closeFileDiff: () => dispatch({ type: 'fileDiffClosed' }),
    commitState,
    commits,
    fileDiffState,
    history,
    pageWidthClassName,
    repoId,
    selectAudience,
    selectCommit,
    selectFile: (file: CommitFile) =>
      dispatch({ path: file.path, type: 'fileSelected' }),
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
                'grid w-full grid-cols-[minmax(0,1fr)_80px] gap-3 px-2 py-3 text-left text-sm transition-colors hover:bg-muted/70',
                selected &&
                  'bg-brand-muted shadow-[inset_2px_0_0_0_var(--brand)] hover:bg-brand-muted',
              )}
              key={commit.projected_id}
              onClick={() => onSelectCommit(commit)}
              type="button"
            >
              <div className="min-w-0">
                <div className="flex min-w-0 items-center gap-2">
                  <GitCommit className="size-4 shrink-0 text-muted-foreground" />
                  <span className="truncate font-mono text-xs font-medium">
                    {commitTitle(commit)}
                  </span>
                </div>
                <div className="mt-1 flex flex-wrap gap-2 pl-6 text-xs leading-4 text-muted-foreground">
                  <span>{commit.logical_commit_id}</span>
                  {commit.author && <span>{commit.author}</span>}
                </div>
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
  fileDiffState,
  onCloseFileDiff,
  onSelectFile,
  selectedFilePath,
}: {
  commitState: CommitDetailState
  fileDiffState: CommitFileDiffState
  onCloseFileDiff: () => void
  onSelectFile: (file: CommitFile) => void
  selectedFilePath: string | null
}) {
  if (commitState.status === 'loading') {
    return <CommitDetailSkeleton />
  }

  if (commitState.status === 'failed') {
    return (
      <PanelState tone="error">
        <TriangleAlert className="size-4 text-destructive" />
        <span>{commitState.error}</span>
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

      <div
        className={cn(
          'grid grid-cols-1 transition-[grid-template-columns] duration-300 ease-out xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]',
          !diffOpen && 'xl:grid-cols-[minmax(0,1fr)_minmax(0,0fr)]',
        )}
      >
        <div className="min-w-0">
          {commit.files.length === 0 ? (
            <div className="px-2 py-10 text-sm text-muted-foreground">
              No file changes in this commit.
            </div>
          ) : (
            <ReviewTree
              compactVisibility
              files={commit.files}
              onSelectFile={(file) => onSelectFile(file as CommitFile)}
              pendingKey={null}
              selectedFilePath={selectedFilePath}
              stagedReview
            />
          )}
        </div>
        <div
          className={cn(
            'min-w-0 overflow-hidden border-border transition-[max-height,opacity,transform,border-color] duration-300 ease-out xl:border-l',
            diffOpen
              ? 'max-h-[70vh] translate-y-0 opacity-100 xl:max-h-none xl:translate-x-0'
              : 'pointer-events-none max-h-0 -translate-y-1 border-transparent opacity-0 xl:translate-x-3',
          )}
        >
          {diffOpen ? (
            <ReviewFileDiffDrawer
              diff={fileDiffState.diff}
              error={fileDiffState.error}
              loading={fileDiffState.status === 'loading'}
              onClose={onCloseFileDiff}
              selectedPath={selectedFilePath}
            />
          ) : null}
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
      {(['owner', 'public'] as const).map((option) => {
        const Icon = option === 'owner' ? UserRound : Globe2
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

type HistoryPageState = {
  audience: ProjectionPreviewAudience
  commitState: CommitDetailState
  fileDiffState: CommitFileDiffState
  selectedCommitId: string | null
  selectedFilePath: string | null
}

type HistoryPageStateInput = Pick<
  HistoryPageProps,
  'histories' | 'initialAudience' | 'initialCommit'
>

type HistoryPageAction =
  | { state: HistoryPageState; type: 'stateReset' }
  | {
      audience: ProjectionPreviewAudience
      commitId: string | null
      type: 'audienceSelected'
    }
  | { commitId: string; type: 'commitSelected' }
  | {
      commit: CommitDetail | null
      loading: boolean
      type: 'commitRequestChanged'
    }
  | { commit: CommitDetail; type: 'commitLoaded' }
  | { message: string; type: 'commitFailed' }
  | { path: string; type: 'fileSelected' }
  | { type: 'fileDiffClosed' }
  | { loading: boolean; type: 'fileDiffRequestChanged' }
  | { diff: ReviewFileDiff; type: 'fileDiffLoaded' }
  | { message: string; type: 'fileDiffFailed' }

function createHistoryPageState({
  histories,
  initialAudience,
  initialCommit,
}: HistoryPageStateInput): HistoryPageState {
  return {
    audience: initialAudience,
    commitState: initialCommit
      ? { commit: initialCommit, error: null, status: 'loaded' }
      : emptyCommitState,
    fileDiffState: emptyFileDiffState,
    selectedCommitId:
      initialCommit?.projected_id ??
      latestCommitId(histories[initialAudience]) ??
      null,
    selectedFilePath: null,
  }
}

function historyPageReducer(
  state: HistoryPageState,
  action: HistoryPageAction,
): HistoryPageState {
  switch (action.type) {
    case 'stateReset':
      return action.state
    case 'audienceSelected':
      return {
        ...state,
        audience: action.audience,
        selectedCommitId: action.commitId,
        selectedFilePath: null,
      }
    case 'commitSelected':
      return {
        ...state,
        selectedCommitId: action.commitId,
        selectedFilePath: null,
      }
    case 'commitRequestChanged':
      return {
        ...state,
        commitState: action.commit
          ? { commit: action.commit, error: null, status: 'loaded' }
          : action.loading
            ? { commit: null, error: null, status: 'loading' }
            : emptyCommitState,
        fileDiffState: emptyFileDiffState,
        selectedFilePath: null,
      }
    case 'commitLoaded':
      return {
        ...state,
        commitState: { commit: action.commit, error: null, status: 'loaded' },
      }
    case 'commitFailed':
      return {
        ...state,
        commitState: {
          commit: null,
          error: action.message,
          status: 'failed',
        },
      }
    case 'fileSelected':
      return {
        ...state,
        selectedFilePath:
          state.selectedFilePath === action.path ? null : action.path,
      }
    case 'fileDiffClosed':
      return { ...state, selectedFilePath: null }
    case 'fileDiffRequestChanged':
      return {
        ...state,
        fileDiffState: action.loading
          ? { diff: null, error: null, status: 'loading' }
          : emptyFileDiffState,
      }
    case 'fileDiffLoaded':
      return {
        ...state,
        fileDiffState: { diff: action.diff, error: null, status: 'loaded' },
      }
    case 'fileDiffFailed':
      return {
        ...state,
        fileDiffState: {
          diff: null,
          error: action.message,
          status: 'failed',
        },
      }
  }
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

const emptyFileDiffState: CommitFileDiffState = {
  diff: null,
  error: null,
  status: 'idle',
}

const emptyCommitState: CommitDetailState = {
  commit: null,
  error: null,
  status: 'idle',
}

function commitRequestKey(
  audience: ProjectionPreviewAudience,
  projectedId: string,
) {
  return `${audience}:${projectedId}`
}

function fileDiffRequestKey(
  audience: ProjectionPreviewAudience,
  projectedId: string,
  path: string,
) {
  return `${audience}:${projectedId}:${path}`
}

function latestCommitId(history: CommitHistory | null) {
  return history?.commits.at(-1)?.projected_id ?? null
}

function commitTitle(commit: Pick<CommitSummary, 'message'>) {
  return commit.message.split(/\r?\n/, 1)[0]?.trim() || '(no message)'
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
