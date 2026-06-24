import {
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  loadJson,
  readRequestAuthToken,
  HttpError,
} from '@/api/client'
import type {
  PendingImportPayload,
  RepoPublicationState,
  RepoParams,
  ReviewFileDiff,
  ReviewFileDiffInput,
  RepoReviewResult,
  SetVisibilityInput,
  StagedUpdate,
} from './types'
import { parseRepoParams } from './repo-detail'

export async function loadReviewForRequest(data: RepoParams) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to review this import.')
  }

  return loadRepoReview(data, idToken)
}

export async function loadReviewFileDiffForRequest(data: ReviewFileDiffInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to review this file.')
  }

  const query = new URLSearchParams({ path: data.path })
  return loadJson<ReviewFileDiff>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/review/file-diff?${query}`,
    { headers: authHeaders(idToken) },
  )
}

export async function setVisibilityForRequest(data: SetVisibilityInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to update file visibility.')
  }

  if (data.kind === 'StagedUpdate') {
    await updateVisibility(idToken, reviewVisibilityUrl(data), {
      paths: data.paths,
      visibility: data.visibility,
    })
  } else {
    await updateVisibility(idToken, reviewVisibilityUrl(data), {
      paths: data.paths,
      visibility: data.visibility,
    })
  }

  const updated = await loadRepoReview(data, idToken)
  if (updated.kind === 'NoReview') {
    throw new Error('No review is waiting for this repo.')
  }

  return updated
}

async function updateVisibility(
  idToken: string,
  url: string,
  body: Record<string, unknown>,
) {
  const response = await fetch(url, {
    body: JSON.stringify(body),
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

export async function publishRepoForRequest(data: RepoParams) {
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
}

export async function postStagedUpdateAction(
  data: RepoParams,
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

async function loadRepoReview(
  data: RepoParams,
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
    line_diff: staged.line_diff,
    files: staged.files,
  }
}

function reviewVisibilityUrl(data: SetVisibilityInput) {
  const endpoint =
    data.kind === 'StagedUpdate'
      ? 'staged-update/files/visibility'
      : 'files/visibility'

  return `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/${endpoint}`
}

export function parseSetVisibilityInput(input: unknown): SetVisibilityInput {
  const data = input as Partial<SetVisibilityInput> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''
  const kind = data?.kind === 'StagedUpdate' ? 'StagedUpdate' : 'PendingImport'
  const paths = Array.isArray(data?.paths)
    ? data.paths.flatMap((path) => {
        if (typeof path !== 'string') {
          return []
        }

        const trimmed = path.trim()
        return trimmed ? [trimmed] : []
      })
    : []
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!owner || !repo) {
    throw new Error('Repository route is incomplete.')
  }

  if (paths.length === 0) {
    throw new Error('At least one file path is required.')
  }

  return { owner, repo, kind, paths, visibility }
}

export function parseReviewFileDiffInput(input: unknown): ReviewFileDiffInput {
  const params = parseRepoParams(input)
  const data = input as Partial<ReviewFileDiffInput> | null
  const path = typeof data?.path === 'string' ? data.path.trim() : ''

  if (!path) {
    throw new Error('A file path is required.')
  }

  return { ...params, path }
}
