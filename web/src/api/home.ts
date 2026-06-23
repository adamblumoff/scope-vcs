import {
  HttpError,
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import { authCookieName } from '@/lib/auth'
import type {
  AccountSession,
  CreateRepoInput,
  CreateRepoResponse,
  HomeState,
  RepoSummary,
} from './types'

export async function loadHomeForRequest(): Promise<HomeState> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    return {
      account: null,
      error: null,
      repositories: [],
      signedIn: false,
    }
  }

  try {
    const api = getApiConnection()
    const init = { headers: authHeaders(idToken) }
    const [account, repositories] = await Promise.all([
      loadJson<AccountSession>(`${api}/v1/session`, init),
      loadJson<RepoSummary[]>(`${api}/v1/repos`, init),
    ])

    return {
      account,
      error: null,
      repositories,
      signedIn: true,
    }
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) {
      const { deleteCookie } = await import('@tanstack/react-start/server')
      deleteCookie(authCookieName, { path: '/' })
      return {
        account: null,
        error: null,
        repositories: [],
        signedIn: false,
      }
    }

    return {
      account: null,
      error: error instanceof Error ? error.message : 'request failed',
      repositories: [],
      signedIn: true,
    }
  }
}

export async function createRepoForRequest(data: CreateRepoInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in to create a repository.')
  }

  const response = await fetch(`${getApiMutationConnection()}/v1/repos`, {
    body: JSON.stringify(data),
    headers: {
      ...authHeaders(idToken),
      'content-type': 'application/json',
    },
    method: 'POST',
  })
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as CreateRepoResponse
}

export function parseCreateRepoInput(input: unknown): CreateRepoInput {
  const data = input as Partial<CreateRepoInput> | null
  const name = typeof data?.name === 'string' ? data.name.trim() : ''
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!name) {
    throw new Error('Repository name is required.')
  }

  return { name, visibility }
}
