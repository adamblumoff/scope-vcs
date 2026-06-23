import type {
  RepoDetail,
  RepoParams,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { RouteErrorPage } from '@/components/route-error-page'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useRouter } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { useReducer } from 'react'
import { ReviewVisibilityPanel } from '../review/review-visibility-panel'
import {
  initialRepoDetailPageState,
  repoDetailPageReducer,
} from './repo-detail-state'

export function RepoDetailPage({
  detail,
  params,
  setFileVisibility,
}: {
  detail: RepoDetail
  params: RepoParams
  setFileVisibility: (
    params: RepoParams,
    files: ReviewFile[],
    visibility: Visibility,
  ) => Promise<ReviewFile[]>
}) {
  const router = useRouter()
  const { repo } = detail
  const [state, dispatch] = useReducer(
    repoDetailPageReducer,
    initialRepoDetailPageState,
  )
  const {
    filesOverride,
    pendingVisibility,
    visibilityError,
  } = state
  const baseFiles = detail.review?.files ?? detail.files
  const files =
    filesOverride?.baseFiles === baseFiles
      ? filesOverride.files
      : baseFiles
  const pendingKey =
    pendingVisibility?.baseFiles === baseFiles
      ? pendingVisibility.key
      : null
  const error =
    visibilityError?.baseFiles === baseFiles
      ? visibilityError.message
      : null
  const canEditFiles = detail.capabilities.write && repo.role === 'Owner'
  const publicOnlyView = repo.role === null
  const stagedReview = detail.review?.kind === 'StagedUpdate'

  async function setVisibility(
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) {
    if (files.length === 0) {
      return
    }

    dispatch({ baseFiles, key: pendingKey, type: 'visibilityStarted' })
    try {
      const updated = await setFileVisibility(params, files, visibility)
      dispatch({
        baseFiles,
        files: updated,
        type: 'visibilitySucceeded',
      })
      await router.invalidate()
    } catch (visibilityError) {
      dispatch({
        baseFiles,
        message:
          visibilityError instanceof Error
            ? visibilityError.message
            : 'visibility update failed',
        type: 'visibilityFailed',
      })
    } finally {
      dispatch({ type: 'visibilityFinished' })
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={repo.id} subtitleClassName="font-mono" />

      <PageContent>
        <PageHeader
          actions={() => (
            <>
              <RepoPrimaryActionButton
                includeOpen={false}
                repo={repo}
                requireOwner
                variant="default"
              />
              {repo.role === 'Owner' && (
                <Button asChild size="sm" variant="secondary">
                  <Link
                    params={{ owner: repo.owner_handle, repo: repo.name }}
                    to="/repos/$owner/$repo/settings"
                  >
                    <Settings className="size-3.5" />
                    <span>Settings</span>
                  </Link>
                </Button>
              )}
            </>
          )}
          badges={() => (
            <>
              <Badge variant="outline">{repo.lifecycle_state}</Badge>
              {repo.role === 'Owner' && (
                <VisibilityBadge visibility={repo.default_visibility} />
              )}
              <Badge variant="outline">{files.length} files</Badge>
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
              )}
            </>
          )}
          title={repo.id}
          titleClassName="font-mono"
        />

        {error && (
          <PageErrorAlert title="Visibility update failed">
            {error}
          </PageErrorAlert>
        )}

        <ReviewVisibilityPanel
          description={
            publicOnlyView
              ? 'Public files and history available to signed-out readers.'
              : canEditFiles
                ? 'Set public or private access in the tree. Switch views to see which rows that audience receives.'
                : 'Files and history visible to your current session.'
          }
          emptyDescription={
            publicOnlyView
              ? 'This repo does not expose any public files yet.'
              : repo.lifecycle_state === 'PendingPublish'
                ? 'Review the pending import before publishing.'
                : 'Files will appear here after the repo has published content.'
          }
          emptyTitle={publicOnlyView ? 'No public files' : 'No live files'}
          files={files}
          onSetVisibility={
            canEditFiles
              ? (files, visibility, key) =>
                  void setVisibility(files, visibility, key)
              : undefined
          }
          pendingKey={pendingKey}
          previews={detail.projection_previews}
          showPrivateCounts={canEditFiles}
          stagedReview={stagedReview}
          title={publicOnlyView ? 'Public files' : 'Visibility'}
          treeVariant={publicOnlyView ? 'public' : 'workflow'}
        />
      </PageContent>
    </main>
  )
}

export function RepoDetailError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected repository error"
      title="Repository unavailable"
    />
  )
}
