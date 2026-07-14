import { HttpError, createApiClient } from '@/api/client'
import { buildCliInstallCommands } from '@/api/cli-install'
import type { AccountSession, HomeState, RepoSummary } from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'

export async function loadHomeForRequest(): Promise<HomeState> {
  const cliInstallCommands = buildCliInstallCommands()
  const api = createApiClient()

  try {
    const [account, repositories] = await Promise.all([
      api.get<AccountSession>(buildApiPath(ApiRouteTemplates.accountSession), {
        auth: 'required',
      }),
      api.get<RepoSummary[]>(buildApiPath(ApiRouteTemplates.repos), {
        auth: 'required',
      }),
    ])

    return {
      account,
      cliInstallCommands,
      error: null,
      repositories,
    }
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) {
      throw new Error('Sign in required.')
    }

    return {
      account: null,
      cliInstallCommands,
      error: error instanceof Error ? error.message : 'request failed',
      repositories: [],
    }
  }
}
