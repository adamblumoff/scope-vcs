import {
  HttpError,
  authHeaders,
  getApiConnection,
  loadJson,
  readRequestAuthToken,
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
  options: { includeOwner?: boolean } = {},
): Promise<ProjectionPreviews> {
  const idToken = await readRequestAuthToken()
  const includeOwner = options.includeOwner ?? true
  const [owner, publicPreview] = await Promise.all([
    idToken && includeOwner
      ? loadProjectionPreviewWithToken({ ...data, audience: 'owner', source }, idToken)
      : Promise.resolve(null),
    loadOptionalProjectionPreviewWithToken(
      { ...data, audience: 'public', source },
      idToken,
    ),
  ])

  return {
    owner,
    public: publicPreview,
    source,
  }
}

async function loadProjectionPreviewWithToken(
  data: ProjectionPreviewInput,
  idToken: string | null | undefined,
) {
  const query = new URLSearchParams({
    audience: data.audience,
    source: data.source,
  })

  return loadJson<ProjectionPreview>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/projection-preview?${query}`,
    { headers: authHeaders(idToken) },
  )
}

async function loadOptionalProjectionPreviewWithToken(
  data: ProjectionPreviewInput,
  idToken: string | null | undefined,
) {
  try {
    return await loadProjectionPreviewWithToken(data, idToken)
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) {
      return null
    }
    throw error
  }
}
