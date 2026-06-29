import {
  type ApiClient,
  createApiClient,
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
import { parseRepoParams } from './repo-params'

export async function loadReviewForRequest(
  data: RepoParams,
  api = createApiClient(),
) {
  return loadRepoReview(data, api)
}

export async function loadReviewFileDiffForRequest(data: ReviewFileDiffInput) {
  const query = new URLSearchParams({ path: data.path })
  return createApiClient().get<ReviewFileDiff>(
    `/v1/repos/${data.owner}/${data.repo}/review/file-diff?${query}`,
    { auth: 'required' },
  )
}

export async function setVisibilityForRequest(data: SetVisibilityInput) {
  const api = createApiClient()
  await updateVisibility(api, reviewVisibilityPath(data), {
    paths: data.paths,
    visibility: data.visibility,
  })

  const updated = await loadRepoReview(data, api)
  if (updated.kind === 'NoReview') {
    throw new Error('No review is waiting for this repo.')
  }

  return updated
}

async function updateVisibility(
  api: ApiClient,
  path: string,
  body: Record<string, unknown>,
) {
  await api.patch<unknown>(path, {
    auth: 'required',
    body,
  })
}

export async function publishRepoForRequest(data: RepoParams) {
  return createApiClient().post<{
    id: string
    publication_state: RepoPublicationState
  }>(
    `/v1/repos/${data.owner}/${data.repo}/publish`,
    { auth: 'required' },
  )
}

export async function postStagedUpdateAction(
  data: RepoParams,
  action: 'apply' | 'reject',
) {
  return createApiClient().post<StagedUpdate>(
    `/v1/repos/${data.owner}/${data.repo}/staged-update/${action}`,
    { auth: 'required' },
  )
}

async function loadRepoReview(
  data: RepoParams,
  api: ApiClient,
): Promise<RepoReviewResult> {
  try {
    const pending = await api.get<PendingImportPayload>(
      `/v1/repos/${data.owner}/${data.repo}/pending-import`,
      { auth: 'required' },
    )
    return { kind: 'PendingImport', ...pending }
  } catch (error) {
    if (!(error instanceof HttpError) || error.status !== 400) {
      throw error
    }
  }

  const staged = await api.get<StagedUpdate | null>(
    `/v1/repos/${data.owner}/${data.repo}/staged-update`,
    { auth: 'required' },
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

function reviewVisibilityPath(data: SetVisibilityInput) {
  const endpoint =
    data.kind === 'StagedUpdate'
      ? 'staged-update/files/visibility'
      : 'files/visibility'

  return `/v1/repos/${data.owner}/${data.repo}/${endpoint}`
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
