import {
  type ApiClient,
  createApiClient,
  HttpError,
} from './client'
import type {
  ProjectionPreview,
  ProjectionPreviewInput,
  ProjectionPreviewSource,
  ProjectionPreviews,
  RepoParams,
} from './types'

export async function loadProjectionPreviewsForRequest(
  data: RepoParams,
  source: ProjectionPreviewSource,
  options: { includePrivate?: boolean; api?: ApiClient } = {},
): Promise<ProjectionPreviews> {
  const api = options.api ?? createApiClient()
  const authenticated = await api.authenticated()
  const includePrivate = options.includePrivate ?? true
  const [privatePreview, publicPreview] = await Promise.all([
    authenticated && includePrivate
      ? loadProjectionPreview(api, { ...data, audience: 'private', source })
      : Promise.resolve(null),
    loadOptionalProjectionPreview(api, { ...data, audience: 'public', source }),
  ])

  return {
    private: privatePreview,
    public: publicPreview,
    source,
  }
}

async function loadProjectionPreview(
  api: ApiClient,
  data: ProjectionPreviewInput,
) {
  const query = new URLSearchParams({
    audience: data.audience,
    source: data.source,
  })

  return api.get<ProjectionPreview>(
    `/v1/repos/${data.owner}/${data.repo}/projection-preview?${query}`,
    { auth: 'optional' },
  )
}

async function loadOptionalProjectionPreview(
  api: ApiClient,
  data: ProjectionPreviewInput,
) {
  try {
    return await loadProjectionPreview(api, data)
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) {
      return null
    }
    throw error
  }
}
