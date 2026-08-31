import { createApiClient } from '@/api/client'
import { buildCliInstallCommands } from '@/api/cli-install'
import type { ProfileState } from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'

export async function loadOwnerProfileForRequest(
  handle: string,
): Promise<ProfileState> {
  const api = createApiClient()
  const [account, profile] = await Promise.all([
    api.get(
      buildApiPath(ApiRouteTemplates.accountSession),
      apiValidators.AccountSessionResponse,
      { auth: 'optional' },
    ),
    api.get(
      buildApiPath(ApiRouteTemplates.ownerRepositories, { handle }),
      apiValidators.OwnerProfileResponse,
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
  return api.get(
    buildApiPath(ApiRouteTemplates.accountSession),
    apiValidators.AccountSessionResponse,
    { auth: 'required' },
  )
}
