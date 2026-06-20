import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { authCookieName } from '@/lib/auth'
import { cn } from '@/lib/utils'
import {
  Link,
  createFileRoute,
  redirect,
  useNavigate,
  useRouter,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  AlertCircle,
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  File,
  FileSearch,
  Folder,
  FolderOpen,
  GitBranch,
  Globe2,
  LoaderCircle,
  Lock,
  Minus,
  Rocket,
  X,
} from 'lucide-react'
import { useState } from 'react'

type Visibility = 'Private' | 'Public'
type VisibilityState = Visibility | 'Mixed'
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

type RepoReviewResult = RepoReview | { kind: 'NoReview' }

type ReviewFile = RepoFile | StagedFile

type ReviewParams = {
  owner: string
  repo: string
}

type SetVisibilityInput = ReviewParams & {
  kind: ReviewKind
  paths: string[]
  visibility: Visibility
}

type ReviewTreeNode = {
  children: ReviewTreeNode[]
  files: ReviewFile[]
  key: string
  name: string
  path: string
  type: 'folder'
} | {
  file: ReviewFile
  key: string
  name: string
  path: string
  type: 'file'
}

const localApiBase = 'http://localhost:8080'
const homeFlashKey = 'scope:home-flash'

const loadReviewForRequest = createServerFn({ method: 'GET' })
  .validator(parseReviewParams)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to review this import.')
    }

    return loadRepoReview(data, idToken)
  })

const setVisibilityForRequest = createServerFn({ method: 'POST' })
  .validator(parseSetVisibilityInput)
  .handler(async ({ data }) => {
    const idToken = await readRequestAuthToken()
    if (!idToken) {
      throw new Error('Sign in as the repo owner to update file visibility.')
    }

    for (const path of data.paths) {
      const response = await fetch(reviewVisibilityUrl(data), {
        body: JSON.stringify({
          path,
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
    }

    const updated = await loadRepoReview(data, idToken)
    if (updated.kind === 'NoReview') {
      throw new Error('No review is waiting for this repo.')
    }

    return updated
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
): Promise<RepoReviewResult> {
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

  if (!staged) {
    return { kind: 'NoReview' }
  }

  return {
    kind: 'StagedUpdate',
    publication_state: 'Published',
    default_visibility: null,
    id: staged.id,
    branch: staged.branch,
    base_live_commit_id: staged.base_live_commit_id,
    message: staged.message,
    files: staged.files,
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
  loader: async ({ params }) => {
    const review = await loadReviewForRequest({ data: params })
    if (review.kind === 'NoReview') {
      throw redirect({
        params,
        to: '/repos/$owner/$repo',
      })
    }

    return review
  },
  errorComponent: ReviewError,
  component: ReviewPage,
})

function ReviewPage() {
  const initialReview = Route.useLoaderData()
  const params = Route.useParams()
  const navigate = useNavigate()
  const router = useRouter()
  const [review, setReview] = useState<RepoReview>(initialReview)
  const [pendingKey, setPendingKey] = useState<string | null>(null)
  const [publishing, setPublishing] = useState(false)
  const [rejecting, setRejecting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const stagedReview = review.kind === 'StagedUpdate'

  async function setVisibility(
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) {
    const paths = files.map((file) => file.path)
    if (paths.length === 0) {
      return
    }

    setError(null)
    setPendingKey(pendingKey)
    try {
      const updated = await setVisibilityForRequest({
        data: { ...params, kind: review.kind, paths, visibility },
      })
      setReview(updated)
    } catch (visibilityError) {
      setError(
        visibilityError instanceof Error
          ? visibilityError.message
          : 'visibility update failed',
      )
    } finally {
      setPendingKey(null)
    }
  }

  async function completeReview() {
    setPublishing(true)
    setError(null)
    try {
      if (review.kind === 'StagedUpdate') {
        await applyStagedUpdateForRequest({ data: params })
        storeHomeFlash(`${params.owner}/${params.repo} update applied.`)
      } else {
        await publishRepoForRequest({ data: params })
        storeHomeFlash(`${params.owner}/${params.repo} published.`)
      }
      await navigate({ replace: true, to: '/' })
      await router.invalidate()
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
      storeHomeFlash(`${params.owner}/${params.repo} update rejected.`)
      await navigate({ replace: true, to: '/' })
      await router.invalidate()
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
            <ReviewTree
              disabled={publishing || rejecting}
              files={review.files}
              onSetVisibility={(files, visibility, key) =>
                void setVisibility(files, visibility, key)
              }
              pendingKey={pendingKey}
              stagedReview={stagedReview}
            />
          )}
        </section>
      </section>
    </main>
  )
}

function ReviewTree({
  disabled,
  files,
  onSetVisibility,
  pendingKey,
  stagedReview,
}: {
  disabled: boolean
  files: ReviewFile[]
  onSetVisibility: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  pendingKey: string | null
  stagedReview: boolean
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set())
  const root = buildReviewTree(files)

  function toggleFolder(key: string) {
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }

  return (
    <div className="divide-y divide-border">
      <div className="hidden grid-cols-[minmax(0,1fr)_110px_120px_120px] gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid">
        <div>Path</div>
        <div>{stagedReview ? 'Change' : 'Scope'}</div>
        <div>Visibility</div>
        <div className="text-right">Set</div>
      </div>
      {root.children.map((node) => (
        <ReviewTreeNodeRow
          collapsed={collapsed}
          depth={0}
          disabled={disabled}
          key={node.key}
          node={node}
          onSetVisibility={onSetVisibility}
          onToggleFolder={toggleFolder}
          pendingKey={pendingKey}
          stagedReview={stagedReview}
        />
      ))}
    </div>
  )
}

function ReviewTreeNodeRow({
  collapsed,
  depth,
  disabled,
  node,
  onSetVisibility,
  onToggleFolder,
  pendingKey,
  stagedReview,
}: {
  collapsed: Set<string>
  depth: number
  disabled: boolean
  node: ReviewTreeNode
  onSetVisibility: (
    files: ReviewFile[],
    visibility: Visibility,
    pendingKey: string,
  ) => void
  onToggleFolder: (key: string) => void
  pendingKey: string | null
  stagedReview: boolean
}) {
  if (node.type === 'file') {
    const nextVisibility = node.file.visibility === 'Public' ? 'Private' : 'Public'
    const busy = pendingKey === node.key
    return (
      <div className="grid gap-2 px-2 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_110px_120px_120px] sm:items-center">
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          <span className="size-6 shrink-0" />
          <File className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {displayPath(node.path)}
          </span>
        </div>
        <div>
          {stagedReview && (
            <Badge variant="outline">
              {'kind' in node.file ? node.file.kind : 'Modified'}
            </Badge>
          )}
        </div>
        <div>
          <VisibilityBadge visibility={node.file.visibility} />
        </div>
        <div className="flex justify-end">
          <Button
            aria-label={`Set ${displayPath(node.path)} ${nextVisibility.toLowerCase()}`}
            disabled={disabled || busy || pendingKey !== null}
            onClick={() => onSetVisibility([node.file], nextVisibility, node.key)}
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
        </div>
      </div>
    )
  }

  const isCollapsed = collapsed.has(node.key)
  const visibility = folderVisibility(node.files)
  const nextVisibility = visibility === 'Public' ? 'Private' : 'Public'
  const busy = pendingKey === node.key

  return (
    <>
      <div className="grid gap-2 bg-muted/20 px-2 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_110px_120px_120px] sm:items-center">
        <div
          className="flex min-w-0 items-center gap-2"
          style={{ paddingLeft: `${depth * 18}px` }}
        >
          <Button
            aria-label={`${isCollapsed ? 'Expand' : 'Collapse'} ${node.name}`}
            onClick={() => onToggleFolder(node.key)}
            size="icon-xs"
            type="button"
            variant="secondary"
          >
            {isCollapsed ? (
              <ChevronRight className="size-3" />
            ) : (
              <ChevronDown className="size-3" />
            )}
          </Button>
          {isCollapsed ? (
            <Folder className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <FolderOpen className="size-4 shrink-0 text-muted-foreground" />
          )}
          <span className="min-w-0 truncate font-mono text-xs" title={node.path}>
            {node.name}
          </span>
        </div>
        <div className="text-xs leading-4 text-muted-foreground">
          {node.files.length} {node.files.length === 1 ? 'file' : 'files'}
        </div>
        <div>
          <VisibilityBadge visibility={visibility} />
        </div>
        <div className="flex justify-end">
          <Button
            aria-label={`Set ${node.path} ${nextVisibility.toLowerCase()}`}
            disabled={disabled || busy || pendingKey !== null}
            onClick={() => onSetVisibility(node.files, nextVisibility, node.key)}
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
        </div>
      </div>
      {!isCollapsed &&
        node.children.map((child) => (
          <ReviewTreeNodeRow
            collapsed={collapsed}
            depth={depth + 1}
            disabled={disabled}
            key={child.key}
            node={child}
            onSetVisibility={onSetVisibility}
            onToggleFolder={onToggleFolder}
            pendingKey={pendingKey}
            stagedReview={stagedReview}
          />
        ))}
    </>
  )
}

function buildReviewTree(files: ReviewFile[]) {
  const root: Extract<ReviewTreeNode, { type: 'folder' }> = {
    children: [],
    files: [],
    key: 'folder:/',
    name: '',
    path: '/',
    type: 'folder',
  }

  for (const file of files) {
    const parts = pathParts(file.path)
    let current = root
    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      const path = `/${parts.slice(0, index + 1).join('/')}`
      const last = index === parts.length - 1
      if (last) {
        current.children.push({
          file,
          key: `file:${file.path}`,
          name: part,
          path: file.path,
          type: 'file',
        })
      } else {
        let folder = current.children.find(
          (child): child is Extract<ReviewTreeNode, { type: 'folder' }> =>
            child.type === 'folder' && child.path === path,
        )
        if (!folder) {
          folder = {
            children: [],
            files: [],
            key: `folder:${path}`,
            name: part,
            path,
            type: 'folder',
          }
          current.children.push(folder)
        }
        current = folder
      }
    }
  }

  sortReviewTree(root)
  attachDescendantFiles(root)
  return root
}

function sortReviewTree(node: Extract<ReviewTreeNode, { type: 'folder' }>) {
  node.children.sort((left, right) => {
    if (left.type !== right.type) {
      return left.type === 'folder' ? -1 : 1
    }
    return left.name.localeCompare(right.name)
  })
  for (const child of node.children) {
    if (child.type === 'folder') {
      sortReviewTree(child)
    }
  }
}

function attachDescendantFiles(node: Extract<ReviewTreeNode, { type: 'folder' }>) {
  node.files = node.children.flatMap((child) => {
    if (child.type === 'file') {
      return [child.file]
    }
    attachDescendantFiles(child)
    return child.files
  })
}

function folderVisibility(files: ReviewFile[]): VisibilityState {
  const hasPublic = files.some((file) => file.visibility === 'Public')
  const hasPrivate = files.some((file) => file.visibility === 'Private')
  if (hasPublic && hasPrivate) {
    return 'Mixed'
  }
  return hasPublic ? 'Public' : 'Private'
}

function pathParts(path: string) {
  return path.replace(/^\/+/, '').split('/').filter(Boolean)
}

function displayPath(path: string) {
  return path.replace(/^\/+/, '')
}

function VisibilityBadge({ visibility }: { visibility: VisibilityState }) {
  return (
    <Badge
      className={cn(
        visibility === 'Private' && 'border-amber-400 bg-amber-100 text-amber-900',
        visibility === 'Public' && 'border-green-400 bg-green-100 text-green-900',
        visibility === 'Mixed' && 'border-blue-400 bg-blue-100 text-blue-900',
      )}
      variant="outline"
    >
      {visibility === 'Mixed' ? (
        <Minus className="size-3" />
      ) : visibility === 'Private' ? (
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

function parseSetVisibilityInput(input: unknown): SetVisibilityInput {
  const data = input as Partial<SetVisibilityInput> | null
  const params = parseReviewParams(data)
  const kind = data?.kind === 'StagedUpdate' ? 'StagedUpdate' : 'PendingImport'
  const paths = Array.isArray(data?.paths)
    ? data.paths
        .filter((path): path is string => typeof path === 'string')
        .map((path) => path.trim())
        .filter(Boolean)
    : []
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (paths.length === 0) {
    throw new Error('At least one file path is required.')
  }

  return { ...params, kind, paths, visibility }
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

function reviewVisibilityUrl(data: SetVisibilityInput) {
  const endpoint =
    data.kind === 'StagedUpdate'
      ? 'staged-update/files/visibility'
      : 'files/visibility'

  return `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/${endpoint}`
}

function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

function storeHomeFlash(message: string) {
  if (typeof window === 'undefined') {
    return
  }

  window.sessionStorage.setItem(homeFlashKey, message)
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
