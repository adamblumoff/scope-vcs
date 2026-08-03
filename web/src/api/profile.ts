import { createApiClient } from '@/api/client'
import { buildCliInstallCommands } from '@/api/cli-install'
import type { AccountSession, OwnerProfile, ProfileState } from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'

export async function loadOwnerProfileForRequest(
  handle: string,
): Promise<ProfileState> {
  const api = createApiClient()
  const [account, profile] = await Promise.all([
    api.get<AccountSession>(buildApiPath(ApiRouteTemplates.accountSession), {
      auth: 'optional',
    }),
    api.get<OwnerProfile>(
      buildApiPath(ApiRouteTemplates.ownerRepositories, { handle }),
      { auth: 'optional' },
    ),
  ])

  return {
    account,
    cliInstallCommands: buildCliInstallCommands(),
    profile,
  }
}

export async function loadAuthenticatedAccountForRequest() {
  const api = createApiClient()
  return api.get<AccountSession>(buildApiPath(ApiRouteTemplates.accountSession), {
    auth: 'required',
  })
}
