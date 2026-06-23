import {
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import type {
  DeleteRepoInput,
  DeleteRepoResponse,
  RepoParams,
  RepoSettings,
  UpdateRepoSettingsInput,
} from './types'

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in to delete a repository.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}`,
    {
      headers: authHeaders(idToken),
      method: 'DELETE',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as DeleteRepoResponse
}

export async function loadRepoSettingsForRequest(
  data: RepoParams,
): Promise<RepoSettings> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to load repository settings.')
  }

  return loadJson<RepoSettings>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/settings`,
    { headers: authHeaders(idToken) },
  )
}

export async function updateRepoSettingsForRequest(
  data: UpdateRepoSettingsInput,
): Promise<RepoSettings> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to update repository settings.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/settings`,
    {
      body: JSON.stringify({
        default_new_file_visibility: data.default_new_file_visibility,
        review_pushes_before_applying: data.review_pushes_before_applying,
      }),
      headers: {
        ...authHeaders(idToken),
        'content-type': 'application/json',
      },
      method: 'PATCH',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as RepoSettings
}
