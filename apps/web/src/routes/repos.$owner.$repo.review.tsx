import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { authCookieName } from '@/lib/auth'
import { cn } from '@/lib/utils'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowLeft,
  Check,
  FileSearch,
  GitBranch,
  Globe2,
  LoaderCircle,
  Lock,
  Rocket,
  X,
} from 'lucide-react'
import { useState } from 'react'

type Visibility = 'Private' | 'Public'
type RepoPublicationState = 'PendingFirstPush' | 'PendingPublish' | 'Published'
type ReviewKind = 'PendingImport' | 'StagedUpdate'
type StagedFileChangeKind = 'Added' | 'Modified' | 'Deleted'

type RepoFile = {
  path: string
  oid: string
  tracked: boolean
  visibility: Visibility
}

type PendingImportPayload = {
  publication_state: 'PendingPublish'
  default_visibility: Visibility
  files: RepoFile[]
}

type StagedFile = {
  path: string
  kind: StagedFileChangeKind
  old_oid: string | null
  new_oid: string | null
  visibility: Visibility
}

type StagedUpdate = {
  id: string
  branch: string
  base_live_commit_id: string | null
  message: string
  files: StagedFile[]
}

type PendingImportReview = PendingImportPayload & {
  kind: 'PendingImport'
}

type StagedUpdateReview = {
  kind: 'StagedUpdate'
  publication_state: 'Published'
  default_visibility: null
  id: string | null
  branch: string | null
  base_live_commit_id: string | null
  message: string | null
  files: StagedFile[]
}

type RepoReview = PendingImportReview | StagedUpdateReview

type ReviewParams = {
  owner: string
  repo: string
}

type SetFileVisibilityInput = ReviewParams & {
  kind: ReviewKind
  path: string
  visibility: Visibility
}

const localApiBase = 'http://localhost:8080'

const loadReviewForRequest = createServerFn({ method: 'GET' })
  .validator(parseReviewParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to review this import.')
    }

    return loadRepoReview(data, idToken)
  })

const setFileVisibilityForRequest = createServerFn({ method: 'POST' })
  .validator(parseSetFileVisibilityInput)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to update file visibility.')
    }

    const response = await fetch(reviewVisibilityUrl(data), {
      body: JSON.stringify({
        path: data.path,
        visibility: data.visibility,
      }),
      headers: {
        ...authHeaders(idToken),
        'content-type': 'application/json',
      },
      method: 'PATCH',
    })
    const payload = await response.json().catch(() => null)

    if (!response.ok) {
      throw new Error(payload?.error ?? `request failed: ${response.status}`)
    }

    return payload as RepoFile | StagedUpdate
  })

const publishRepoForRequest = createServerFn({ method: 'POST' })
  .validator(parseReviewParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to publish this repo.')
    }

    const response = await fetch(
      `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/publish`,
      {
        headers: authHeaders(idToken),
        method: 'POST',
      },
    )
    const payload = await response.json().catch(() => null)

    if (!response.ok) {
      throw new Error(payload?.error ?? `request failed: ${response.status}`)
    }

    return payload as { id: string; publication_state: RepoPublicationState }
  })

const applyStagedUpdateForRequest = createServerFn({ method: 'POST' })
  .validator(parseReviewParams)
  .handler(async ({ data }) => postStagedUpdateAction(data, 'apply'))

const rejectStagedUpdateForRequest = createServerFn({ method: 'POST' })
  .validator(parseReviewParams)
  .handler(async ({ data }) => postStagedUpdateAction(data, 'reject'))

async function loadRepoReview(
  data: ReviewParams,
  idToken: string,
): Promise<RepoReview> {
  const api = getApiConnection()
  const init = { headers: authHeaders(idToken) }

  try {
    const pending = await loadJson<PendingImportPayload>(
      `${api}/v1/repos/${data.owner}/${data.repo}/pending-import`,
      init,
    )
    return { kind: 'PendingImport', ...pending }
  } catch (error) {
    if (!(error instanceof HttpError) || error.status !== 400) {
      throw error
    }
  }

  const staged = await loadJson<StagedUpdate | null>(
    `${api}/v1/repos/${data.owner}/${data.repo}/staged-update`,
    init,
  )

  return {
    kind: 'StagedUpdate',
    publication_state: 'Published',
    default_visibility: null,
    id: staged?.id ?? null,
    branch: staged?.branch ?? null,
    base_live_commit_id: staged?.base_live_commit_id ?? null,
    message: staged?.message ?? null,
    files: staged?.files ?? [],
  }
}

async function postStagedUpdateAction(
  data: ReviewParams,
  action: 'apply' | 'reject',
) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to review this push.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/staged-update/${action}`,
    {
      headers: authHeaders(idToken),
      method: 'POST',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as StagedUpdate
}

export const Route = createFileRoute('/repos/$owner/$repo/review')({
  loader: ({ params }) => loadReviewForRequest({ data: params }),
  errorComponent: ReviewError,
  component: ReviewPage,
})

function ReviewPage() {
  const initialReview = Route.useLoaderData()
  const params = Route.useParams()
  const navigate = useNavigate()
  const [review, setReview] = useState<RepoReview>(initialReview)
  const [pendingPath, setPendingPath] = useState<string | null>(null)
  const [publishing, setPublishing] = useState(false)
  const [rejecting, setRejecting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const stagedReview = review.kind === 'StagedUpdate'

  async function setVisibility(file: RepoFile | StagedFile, visibility: Visibility) {
    setError(null)
    setPendingPath(file.path)
    try {
      const updated = await setFileVisibilityForRequest({
        data: { ...params, kind: review.kind, path: file.path, visibility },
      })
      setReview((current) => updateReviewVisibility(current, updated))
    } catch (visibilityError) {
      setError(
        visibilityError instanceof Error
          ? visibilityError.message
          : 'visibility update failed',
      )
    } finally {
      setPendingPath(null)
    }
  }

  async function completeReview() {
    setPublishing(true)
    setError(null)
    try {
      if (review.kind === 'StagedUpdate') {
        await applyStagedUpdateForRequest({ data: params })
      } else {
        await publishRepoForRequest({ data: params })
      }
      await navigate({ to: '/' })
    } catch (publishError) {
      setError(
        publishError instanceof Error ? publishError.message : 'review action failed',
      )
      setPublishing(false)
    }
  }

  async function rejectStagedUpdate() {
    setRejecting(true)
    setError(null)
    try {
      await rejectStagedUpdateForRequest({ data: params })
      await navigate({ to: '/' })
    } catch (rejectError) {
      setError(rejectError instanceof Error ? rejectError.message : 'reject failed')
      setRejecting(false)
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex min-h-16 max-w-[980px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
          <Link className="flex min-w-0 items-center gap-3" to="/">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
              <GitBranch className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate font-mono text-xs leading-4 text-muted-foreground">
                {params.owner}/{params.repo}
              </div>
            </div>
          </Link>
          <Button asChild size="sm" variant="secondary">
            <Link to="/">
              <ArrowLeft className="size-3.5" />
              <span>Repos</span>
            </Link>
          </Button>
        </div>
      </header>

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{review.publication_state}</Badge>
              {review.default_visibility && (
                <VisibilityBadge visibility={review.default_visibility} />
              )}
              <Badge variant="outline">{review.files.length} files</Badge>
              {stagedReview && review.branch && (
                <Badge variant="outline">{review.branch}</Badge>
              )}
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {params.owner}/{params.repo}
            </h1>
          </div>
          <div className="flex items-center gap-2">
            {stagedReview && (
              <Button
                disabled={publishing || rejecting || review.files.length === 0}
                onClick={() => void rejectStagedUpdate()}
                size="sm"
                type="button"
                variant="secondary"
              >
                {rejecting ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <X className="size-3.5" />
                )}
                <span>{rejecting ? 'Rejecting' : 'Reject'}</span>
              </Button>
            )}
            <Button
              disabled={
                publishing || rejecting || (stagedReview && review.files.length === 0)
              }
              onClick={() => void completeReview()}
              size="sm"
              type="button"
            >
              {publishing ? (
                <LoaderCircle className="size-3.5 animate-spin" />
              ) : (
                <Rocket className="size-3.5" />
              )}
              <span>
                {publishing
                  ? stagedReview
                    ? 'Applying'
                    : 'Publishing'
                  : stagedReview
                    ? 'Apply'
                    : 'Publish'}
              </span>
            </Button>
          </div>
        </div>

        {error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Review update failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <section className="mt-8 border-y border-border">
          {review.files.length === 0 ? (
            <div className="flex items-center gap-3 py-10">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
                <FileSearch className="size-5 text-muted-foreground" />
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium leading-5">No files found</div>
                <div className="mt-1 text-sm leading-5 text-muted-foreground">
                  {stagedReview
                    ? 'No staged push is waiting.'
                    : 'This repo can still be published.'}
                </div>
              </div>
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>File</TableHead>
                  {stagedReview && <TableHead className="w-[100px]">Change</TableHead>}
                  <TableHead className="w-[120px]">Visibility</TableHead>
                  <TableHead className="w-[120px] text-right">Change</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {review.files.map((file) => {
                  const nextVisibility =
                    file.visibility === 'Public' ? 'Private' : 'Public'
                  const busy = pendingPath === file.path
                  return (
                    <TableRow key={file.path}>
                      <TableCell className="max-w-[340px] truncate font-mono text-xs sm:max-w-[560px]">
                        {file.path}
                      </TableCell>
                      {stagedReview && (
                        <TableCell>
                          <Badge variant="outline">
                            {'kind' in file ? file.kind : 'Modified'}
                          </Badge>
                        </TableCell>
                      )}
                      <TableCell>
                        <VisibilityBadge visibility={file.visibility} />
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          disabled={busy || publishing || rejecting}
                          onClick={() => void setVisibility(file, nextVisibility)}
                          size="sm"
                          type="button"
                          variant="secondary"
                        >
                          {busy ? (
                            <LoaderCircle className="size-3.5 animate-spin" />
                          ) : (
                            <Check className="size-3.5" />
                          )}
                          <span>{nextVisibility}</span>
                        </Button>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          )}
        </section>
      </section>
    </main>
  )
}

function updateReviewVisibility(
  current: RepoReview,
  updated: RepoFile | StagedUpdate,
): RepoReview {
  if (current.kind === 'StagedUpdate') {
    if (!('files' in updated)) {
      return current
    }

    return {
      ...current,
      id: updated.id,
      branch: updated.branch,
      base_live_commit_id: updated.base_live_commit_id,
      message: updated.message,
      files: updated.files,
    }
  }

  if ('files' in updated) {
    return current
  }

  return {
    ...current,
    files: current.files.map((currentFile) =>
      currentFile.path === updated.path ? updated : currentFile,
    ),
  }
}

function VisibilityBadge({ visibility }: { visibility: Visibility }) {
  return (
    <Badge
      className={cn(
        visibility === 'Private' && 'border-amber-400 bg-amber-100 text-amber-900',
        visibility === 'Public' && 'border-green-400 bg-green-100 text-green-900',
      )}
      variant="outline"
    >
      {visibility === 'Private' ? (
        <Lock className="size-3" />
      ) : (
        <Globe2 className="size-3" />
      )}
      {visibility}
    </Badge>
  )
}

function ReviewError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected review error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] border-y border-border py-6">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Review unavailable</AlertTitle>
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

async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

function parseReviewParams(input: unknown): ReviewParams {
  const data = input as Partial<ReviewParams> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''

  if (!owner || !repo) {
    throw new Error('Repository route is incomplete.')
  }

  return { owner, repo }
}

function parseSetFileVisibilityInput(input: unknown): SetFileVisibilityInput {
  const data = input as Partial<SetFileVisibilityInput> | null
  const params = parseReviewParams(data)
  const kind = data?.kind === 'StagedUpdate' ? 'StagedUpdate' : 'PendingImport'
  const path = typeof data?.path === 'string' ? data.path.trim() : ''
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!path) {
    throw new Error('File path is required.')
  }

  return { ...params, kind, path, visibility }
}

async function loadJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new HttpError(
      payload?.error ?? `request failed: ${response.status}`,
      response.status,
    )
  }

  return payload as T
}

class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message)
  }
}

function reviewVisibilityUrl(data: SetFileVisibilityInput) {
  const endpoint =
    data.kind === 'StagedUpdate'
      ? 'staged-update/files/visibility'
      : 'files/visibility'

  return `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/${endpoint}`
}

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function getApiConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before loading repositories.')
}

function getApiMutationConnection() {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error('Set VITE_SCOPE_API_URL before changing repository state.')
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
