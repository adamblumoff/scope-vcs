import {
  HttpError,
  createApiClient,
  getCliInstallConnection,
} from '@/api/client'
import type {
  AccountSession,
  CliInstallCommands,
  HomeState,
  RepoSummary,
} from './types'
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

function buildCliInstallCommands(): CliInstallCommands {
  const baseUrl = getCliInstallConnection()
  return {
    posix: `curl -fsSL ${baseUrl}/install.sh | sh`,
    windows: `irm ${baseUrl}/install.ps1 | iex`,
  }
}
