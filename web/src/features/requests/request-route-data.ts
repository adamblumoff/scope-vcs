import { HttpError } from '@/api/client'

export function requestParamsForRoute(params: {
  owner: string
  repo: string
  requestId: string
}) {
  return {
    owner: params.owner,
    repo: params.repo,
    request_id: params.requestId,
  }
}

export async function loadOptionalSelectedRequestResource<T>(load: () => Promise<T>) {
  try {
    return await load()
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) return null
    throw error
  }
}
