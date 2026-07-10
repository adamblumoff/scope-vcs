import type {
  CommitDetail,
  CommitFile,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
  ReviewFileDiff,
} from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'
import { useNavigate } from '@tanstack/react-router'
import {
  Globe2,
  GitCommit,
  History,
  LockKeyhole,
  TriangleAlert,
} from 'lucide-react'
import { type ReactNode, useMemo } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import { audienceLabel, changeCountLabel } from '../review/review-labels'

export type CommitHistories = {
  private: CommitHistory | null
  public: CommitHistory | null
}

type HistoryPageProps = {
  histories: CommitHistories
  initialAudience: ProjectionPreviewAudience
  initialCommit: CommitDetail | null
  initialFile: {
    error: string | null
    path: string | null
    value: ReviewFileDiff | null
  }
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
    <RepoShell contentClassName={pageWidthClassName} params={params}>
      <PageContent className={pageWidthClassName}>
        <PageHeader
          badges={(
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
          description={`Projected commit history for ${repoId}.`}
          title="History"
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
                  History appears here once Scope has applied commits.
                </p>
              </div>
            </div>
          ) : (
            <div
              className={cn(
                'grid grid-cols-1 lg:grid-cols-[minmax(260px,0.62fr)_minmax(0,1.38fr)]',
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
    </RepoShell>
  )
}

function useHistoryPageModel({
  histories,
  initialAudience: audience,
  initialCommit,
  initialFile,
  params,
}: HistoryPageProps) {
  const navigate = useNavigate()
  const selectedFilePath = initialFile.path
  const fileDiffState: CommitFileDiffState = initialFile.error
    ? { diff: null, error: initialFile.error, status: 'failed' }
    : initialFile.value
      ? { diff: initialFile.value, error: null, status: 'loaded' }
      : emptyFileDiffState
  const availableAudiences = useMemo(
    () =>
      (['private', 'public'] as const).filter(
        (option) => histories[option] !== null,
      ),
    [histories],
  )
  const history = histories[audience] ?? histories.public ?? histories.private
  const commits = useMemo(
    () => [...(history?.commits ?? [])].reverse(),
    [history?.commits],
  )
  const selectedCommit = initialCommit
  const selectedCommitId = selectedCommit?.projected_id ?? null
  const commitState: CommitDetailState = selectedCommit
    ? { commit: selectedCommit, error: null, status: 'loaded' }
    : emptyCommitState
  const pageWidthClassName = selectedFilePath
    ? 'max-w-[1320px]'
    : 'max-w-[1040px]'
  const repoId = `${params.owner}/${params.repo}`

  function replaceHistorySearch(
    nextAudience: ProjectionPreviewAudience,
    nextCommitId: string | null,
    nextPath: string | null = null,
  ) {
    void navigate({
      params,
      replace: true,
      search: {
        audience: nextAudience,
        commit: nextCommitId ?? undefined,
        path: nextPath ?? undefined,
      },
      to: '/repos/$owner/$repo/history',
    })
  }

  return {
    audience,
    availableAudiences,
    closeFileDiff: () => replaceHistorySearch(audience, selectedCommitId),
    commitState,
    commits,
    fileDiffState,
    history,
    pageWidthClassName,
    repoId,
    selectAudience: (nextAudience: ProjectionPreviewAudience) => {
      const nextHistory = histories[nextAudience]
      if (nextHistory) {
        replaceHistorySearch(nextAudience, latestCommitId(nextHistory))
      }
    },
    selectCommit: (commit: CommitSummary) =>
      replaceHistorySearch(audience, commit.projected_id),
    selectFile: (file: CommitFile) => {
      const nextPath = selectedFilePath === file.path ? null : file.path
      replaceHistorySearch(audience, selectedCommitId, nextPath)
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
          'grid grid-cols-1 xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]',
          !diffOpen && 'xl:grid-cols-[minmax(0,1fr)_minmax(0,0fr)]',
        )}
      >
        <div className="min-w-0">
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
        <div
          className={cn(
            'min-w-0 overflow-hidden border-border xl:border-l',
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


function latestCommitId(history: CommitHistory | null) {
  return history?.commits.at(-1)?.projected_id ?? null
}

function commitTitle(commit: Pick<CommitSummary, 'message'>) {
  return commit.message.split(/\r?\n/, 1)[0]?.trim() || '(no message)'
}

function commitFileStatus(file: CommitFile) {
  return <Badge variant="neutral">{file.kind}</Badge>
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
