import type {
  RepoDetail,
  RepoFile,
  RepoParams,
  RepoSummary,
  ReviewFile,
  Visibility,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ProjectionPreviewPanel } from '@/features/projection-preview/projection-preview-panel'
import { Link, useRouter } from '@tanstack/react-router'
import { AlertCircle, ArrowLeft, ArrowRight, FileSearch } from 'lucide-react'
import { useState } from 'react'
import { ReviewTree } from '../review/review-tree'

type FilesOverride = {
  baseFiles: RepoFile[]
  files: RepoFile[]
}

type PendingVisibility = {
  baseFiles: RepoFile[]
  key: string
}

type VisibilityError = {
  baseFiles: RepoFile[]
  message: string
}

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
  ) => Promise<RepoFile[]>
}) {
  const router = useRouter()
  const { repo } = detail
  const [filesOverride, setFilesOverride] = useState<FilesOverride | null>(
    null,
  )
  const [pendingVisibility, setPendingVisibility] =
    useState<PendingVisibility | null>(null)
  const [visibilityError, setVisibilityError] =
    useState<VisibilityError | null>(null)
  const files =
    filesOverride?.baseFiles === detail.files
      ? filesOverride.files
      : detail.files
  const pendingKey =
    pendingVisibility?.baseFiles === detail.files
      ? pendingVisibility.key
      : null
  const error =
    visibilityError?.baseFiles === detail.files
      ? visibilityError.message
      : null
  const canEditFiles = detail.capabilities.write && repo.role === 'Owner'

  async function setVisibility(
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) {
    if (files.length === 0) {
      return
    }

    setVisibilityError(null)
    setPendingVisibility({ baseFiles: detail.files, key: pendingKey })
    try {
      const updated = await setFileVisibility(params, files, visibility)
      setFilesOverride({ baseFiles: detail.files, files: updated })
      await router.invalidate()
    } catch (visibilityError) {
      setVisibilityError({
        baseFiles: detail.files,
        message:
          visibilityError instanceof Error
            ? visibilityError.message
            : 'visibility update failed',
      })
    } finally {
      setPendingVisibility(null)
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={repo.id} subtitleClassName="font-mono" />

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{repo.lifecycle_state}</Badge>
              {repo.role === 'Owner' && (
                <VisibilityBadge visibility={repo.default_visibility} />
              )}
              <Badge variant="outline">{files.length} files</Badge>
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
              )}
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {repo.id}
            </h1>
          </div>
          <RepoAction repo={repo} />
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Visibility update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <section className="mt-8 border-y border-border">
          <div className="border-b border-border py-4">
            <h2 className="text-sm font-semibold leading-5">Repo files</h2>
            <p className="mt-1 max-w-[660px] text-sm leading-5 text-muted-foreground">
              {canEditFiles
                ? 'Set each file to Private or Public here. The preview below shows the result for each audience.'
                : 'Files visible to your current session.'}
            </p>
          </div>
          {files.length === 0 ? (
            <div className="flex items-center gap-3 py-10">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
                <FileSearch className="size-5 text-muted-foreground" />
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium leading-5">No live files</div>
                <div className="mt-1 text-sm leading-5 text-muted-foreground">
                  {repo.lifecycle_state === 'PendingPublish'
                    ? 'Review the pending import before publishing.'
                    : 'Files will appear here after the repo has published content.'}
                </div>
              </div>
            </div>
          ) : (
            <ReviewTree
              files={files}
              onSetVisibility={
                canEditFiles
                  ? (files, visibility, key) =>
                      void setVisibility(files, visibility, key)
                  : undefined
              }
              pendingKey={pendingKey}
              stagedReview={false}
            />
          )}
        </section>

        <ProjectionPreviewPanel previews={detail.projection_previews} />
      </section>
    </main>
  )
}

function RepoAction({ repo }: { repo: RepoSummary }) {
  if (repo.role !== 'Owner') {
    return null
  }

  if (repo.lifecycle_state === 'PendingFirstPush') {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/setup"
        >
          <ArrowRight className="size-3.5" />
          <span>Setup</span>
        </Link>
      </Button>
    )
  }

  if (repo.lifecycle_state === 'PendingPublish' || repo.staged_update_pending) {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/review"
        >
          <ArrowRight className="size-3.5" />
          <span>Review</span>
        </Link>
      </Button>
    )
  }

  return null
}

export function RepoDetailError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected repository error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] border-y border-border py-6">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Repository unavailable</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
        <Button asChild className="mt-5" size="sm" variant="secondary">
          <Link to="/">
            <ArrowLeft className="size-3.5" />
            <span>Repos</span>
          </Link>
        </Button>
      </div>
    </main>
  )
}
