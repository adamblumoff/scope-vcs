import {
  HttpError,
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  getPublicApiConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import { setupView } from './setup-view'
import type {
  RepoSetup,
  RepoSummary,
  SetupRouteState,
} from './types'

export { setupPushSecretKey } from './setup-token-key'

export async function loadSetupForRequest(data: { owner: string; repo: string }) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to view setup.')
  }

  const api = getApiConnection('loading repository setup')
  try {
    const setup = await loadJson<RepoSetup>(
      `${api}/v1/repos/${data.owner}/${data.repo}/setup`,
      { headers: authHeaders(idToken) },
    )

    return {
      kind: 'setup',
      setup: setupView(
        getPublicApiConnection('building repository setup command'),
        setup,
      ),
    } satisfies SetupRouteState
  } catch (error) {
    if (error instanceof HttpError && error.status === 409) {
      return { kind: 'review' } satisfies SetupRouteState
    }

    throw error
  }
}

export async function loadSetupProgressForRequest(data: {
  owner: string
  repo: string
}) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to check setup progress.')
  }

  const repo = await loadJson<RepoSummary>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}`,
    {
      cache: 'no-store',
      headers: authHeaders(idToken),
    },
  )

  return repo.lifecycle_state
}

export async function regenerateTokenForRequest(data: {
  owner: string
  repo: string
}) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to regenerate setup command.')
  }

  const api = getApiMutationConnection('changing repository setup')
  const response = await fetch(
    `${api}/v1/repos/${data.owner}/${data.repo}/setup-token`,
    {
      headers: authHeaders(idToken),
      method: 'POST',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return setupView(
    getPublicApiConnection('building repository setup command'),
    payload as RepoSetup,
  )
}
