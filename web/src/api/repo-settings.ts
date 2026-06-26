import { createApiClient } from '@/api/client'
import type {
  DeleteRepoInput,
  DeleteRepoResponse,
  RepoParams,
  RepoSettings,
  UpdateRepoSettingsInput,
} from './types'

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  return createApiClient().delete<DeleteRepoResponse>(
    `/v1/repos/${data.owner}/${data.repo}`,
    { auth: 'required' },
  )
}

export async function loadRepoSettingsForRequest(
  data: RepoParams,
): Promise<RepoSettings> {
  return createApiClient().get<RepoSettings>(
    `/v1/repos/${data.owner}/${data.repo}/settings`,
    { auth: 'required' },
  )
}

export async function updateRepoSettingsForRequest(
  data: UpdateRepoSettingsInput,
): Promise<RepoSettings> {
  return createApiClient().patch<RepoSettings>(
    `/v1/repos/${data.owner}/${data.repo}/settings`,
    {
      auth: 'required',
      body: {
        default_new_file_visibility: data.default_new_file_visibility,
        review_pushes_before_applying: data.review_pushes_before_applying,
      },
    },
  )
}
