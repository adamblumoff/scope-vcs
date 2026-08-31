import { createApiClient, noContent } from '@/api/client'
import type {
  CompleteBrowserCliLoginInput,
  CompleteCliLoginInput,
  RevokeCliSessionInput,
} from './cli-login-input'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.cliDeviceLoginComplete, {
      user_code: data.code,
    }),
    apiValidators.DeviceLoginCompleteResponse,
    { auth: 'required' },
  )
}

export async function completeBrowserCliLoginForRequest(
  data: CompleteBrowserCliLoginInput,
) {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.cliBrowserLoginComplete, {
      request_id: data.requestId,
    }),
    apiValidators.BrowserLoginCompleteResponse,
    { auth: 'required' },
  )
}

export async function createCliExchangeGrantForRequest() {
  return createApiClient().post(
    ApiRouteTemplates.cliExchangeGrants,
    apiValidators.CliExchangeGrantResponse,
    { auth: 'required' },
  )
}

export async function listCliSessionsForRequest() {
  return createApiClient().get(
    ApiRouteTemplates.cliSessions,
    apiValidators.CliSessionsResponse,
    { auth: 'required' },
  )
}

export async function revokeCliSessionForRequest(data: RevokeCliSessionInput) {
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.cliSessionById, {
      session_id: data.sessionId,
    }),
    noContent,
    { auth: 'required' },
  )
}
