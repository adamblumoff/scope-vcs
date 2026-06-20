import {
  HttpError,
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  loadJson,
  readRequestAuthToken,
  stripTrailingSlash,
} from '@/api/client'
import type {
  RepoSetup,
  RepoSetupView,
  RepoSummary,
  SetupRouteState,
} from './types'

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

    return { kind: 'setup', setup: setupView(api, setup) } satisfies SetupRouteState
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
    throw new Error('Sign in as the repo owner to regenerate setup token.')
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

  return setupView(api, payload as RepoSetup)
}

export function setupView(api: string, setup: RepoSetup): RepoSetupView {
  const gitRemoteUrl = `${stripTrailingSlash(api)}${setup.git_remote_path}`

  return {
    ...setup,
    git_remote_url: gitRemoteUrl,
  }
}

export function setupSecretKey(repoId: string) {
  return `scope:first-push-token:${repoId}`
}

export function setupPushSecretKey(repoId: string) {
  return `scope:git-push-token:${repoId}`
}
