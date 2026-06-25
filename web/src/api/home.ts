import {
  HttpError,
  authHeaders,
  getCliInstallConnection,
  getApiConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import type {
  AccountSession,
  CliInstallCommands,
  HomeState,
  RepoSummary,
} from './types'

export async function loadHomeForRequest(): Promise<HomeState> {
  const cliInstallCommands = buildCliInstallCommands()
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    return {
      account: null,
      cliInstallCommands,
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
      cliInstallCommands,
      error: null,
      repositories,
      signedIn: true,
    }
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) {
      return {
        account: null,
        cliInstallCommands,
        error: null,
        repositories: [],
        signedIn: false,
      }
    }

    return {
      account: null,
      cliInstallCommands,
      error: error instanceof Error ? error.message : 'request failed',
      repositories: [],
      signedIn: true,
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
