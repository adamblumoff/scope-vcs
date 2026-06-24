import type {
  CommitDetail,
  CommitDetailInput,
  CommitFile,
  CommitFileDiffInput,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
  ReviewFileDiff,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'
import { Link, useNavigate } from '@tanstack/react-router'
import {
  Globe2,
  GitCommit,
  History,
  LoaderCircle,
  TriangleAlert,
  UserRound,
} from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useState } from 'react'
import { ReviewFileDiffDrawer } from '../review/review-file-diff-drawer'
import { ReviewTree } from '../review/review-tree'
import { audienceLabel, changeCountLabel } from '../review/review-labels'

export type CommitHistories = {
  owner: CommitHistory | null
  public: CommitHistory | null
}

export function HistoryPage({
  histories,
  initialAudience,
  initialCommit,
  loadCommit,
  loadFileDiff,
  params,
}: {
  histories: CommitHistories
  initialAudience: ProjectionPreviewAudience
  initialCommit: CommitDetail | null
  loadCommit: (input: CommitDetailInput) => Promise<CommitDetail>
  loadFileDiff: (input: CommitFileDiffInput) => Promise<ReviewFileDiff>
  params: RepoParams
}) {
  const navigate = useNavigate()
  const [audience, setAudience] =
    useState<ProjectionPreviewAudience>(initialAudience)
  const [selectedCommitId, setSelectedCommitId] = useState<string | null>(
    initialCommit?.projected_id ??
      latestCommitId(histories[initialAudience]) ??
      null,
  )
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null)
  const [commitState, setCommitState] = useState<CommitDetailState>(
    initialCommit
      ? { commit: initialCommit, error: null, status: 'loaded' }
      : { commit: null, error: null, status: 'idle' },
  )
  const [fileDiffState, setFileDiffState] =
    useState<CommitFileDiffState>(emptyFileDiffState)
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

  useEffect(() => {
    setAudience(initialAudience)
    setSelectedCommitId(
      initialCommit?.projected_id ??
        latestCommitId(histories[initialAudience]) ??
        null,
    )
    setSelectedFilePath(null)
    setCommitState(
      initialCommit
        ? { commit: initialCommit, error: null, status: 'loaded' }
        : { commit: null, error: null, status: 'idle' },
    )
    setFileDiffState(emptyFileDiffState)
  }, [histories, initialAudience, initialCommit])

  useEffect(() => {
    if (!selectedCommitId || !history) {
      setCommitState({ commit: null, error: null, status: 'idle' })
      return
    }

    if (
      commitState.status === 'loaded' &&
      commitState.commit.audience === audience &&
      commitState.commit.projected_id === selectedCommitId
    ) {
      return
    }

    let active = true
    setCommitState({ commit: null, error: null, status: 'loading' })
    loadCommit({
      audience,
      commit: selectedCommitId,
      owner: params.owner,
      repo: params.repo,
    }).then(
      (commit) => {
        if (active) {
          setCommitState({ commit, error: null, status: 'loaded' })
        }
      },
      (error) => {
        if (active) {
          setCommitState({
            commit: null,
            error:
              error instanceof Error ? error.message : 'commit load failed',
            status: 'failed',
          })
        }
      },
    )

    return () => {
      active = false
    }
  }, [
    audience,
    history,
    loadCommit,
    params.owner,
    params.repo,
    selectedCommitId,
  ])

  useEffect(() => {
    if (!selectedCommit) {
      setSelectedFilePath(null)
      return
    }

    const paths = selectedCommit.files.map((file) => file.path)
    setSelectedFilePath((current) =>
      current && paths.includes(current) ? current : null,
    )
  }, [selectedCommit])

  useEffect(() => {
    if (!selectedCommit || !selectedFilePath) {
      setFileDiffState(emptyFileDiffState)
      return
    }

    let active = true
    setFileDiffState({ diff: null, error: null, status: 'loading' })
    loadFileDiff({
      audience,
      commit: selectedCommit.projected_id,
      owner: params.owner,
      path: selectedFilePath,
      repo: params.repo,
    }).then(
      (diff) => {
        if (active) {
          setFileDiffState({ diff, error: null, status: 'loaded' })
        }
      },
      (error) => {
        if (active) {
          setFileDiffState({
            diff: null,
            error: error instanceof Error ? error.message : 'diff load failed',
            status: 'failed',
          })
        }
      },
    )

    return () => {
      active = false
    }
  }, [
    audience,
    loadFileDiff,
    params.owner,
    params.repo,
    selectedCommit,
    selectedFilePath,
  ])

  function selectAudience(nextAudience: ProjectionPreviewAudience) {
    const nextHistory = histories[nextAudience]
    if (!nextHistory) {
      return
    }

    const nextCommitId = latestCommitId(nextHistory)
    setAudience(nextAudience)
    setSelectedFilePath(null)
    setSelectedCommitId(nextCommitId)
    replaceHistorySearch(nextAudience, nextCommitId)
  }

  function selectCommit(commit: CommitSummary) {
    setSelectedCommitId(commit.projected_id)
    setSelectedFilePath(null)
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

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        contentClassName={pageWidthClassName}
        subtitle={repoId}
        subtitleClassName="font-mono"
      />

      <PageContent className={pageWidthClassName}>
        <PageHeader
          badges={() => (
            <>
              <Badge variant="outline">{audienceLabel(audience)} view</Badge>
              <Badge variant="outline">
                {commits.length} {commits.length === 1 ? 'commit' : 'commits'}
              </Badge>
              {selectedCommit && (
                <Badge variant="outline">
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

        <section className="mt-8 border-y border-border">
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
            <div className="py-10 text-sm text-muted-foreground">
              No commits yet.
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
                onCloseFileDiff={() => setSelectedFilePath(null)}
                onSelectFile={(file) =>
                  setSelectedFilePath((current) =>
                    current === file.path ? null : file.path,
                  )
                }
                selectedFilePath={selectedFilePath}
              />
            </div>
          )}
        </section>
      </PageContent>
    </main>
  )
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
                selected && 'bg-blue-100/60 dark:bg-blue-100/35',
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
                  {commit.synthetic && <span>Synthetic</span>}
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
    return (
      <PanelState>
        <LoaderCircle className="size-4 animate-spin text-muted-foreground" />
        <span>Loading commit</span>
      </PanelState>
    )
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
          <Badge variant="outline">{commit.logical_commit_id}</Badge>
          {commit.synthetic && <Badge variant="outline">Synthetic</Badge>}
          {commit.author && <Badge variant="outline">{commit.author}</Badge>}
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
