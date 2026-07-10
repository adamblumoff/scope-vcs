import { createApiClient } from '@/api/client'
import type {
  CompleteBrowserCliLoginInput,
  CompleteCliLoginInput,
  RevokeCliSessionInput,
} from './cli-login-input'
import type {
  BrowserLoginComplete,
  CliExchangeGrant,
  CliSessions,
} from './types'
import {
  ApiRouteTemplates,
  buildApiPath,
  type DeviceLoginCompleteResponse,
} from './types.generated'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  return createApiClient().post<DeviceLoginCompleteResponse>(
    buildApiPath(ApiRouteTemplates.cliDeviceLoginComplete, {
      user_code: data.code,
    }),
    { auth: 'required' },
  )
}

export async function completeBrowserCliLoginForRequest(
  data: CompleteBrowserCliLoginInput,
) {
  return createApiClient().post<BrowserLoginComplete>(
    buildApiPath(ApiRouteTemplates.cliBrowserLoginComplete, {
      request_id: data.requestId,
    }),
    { auth: 'required' },
  )
}

export async function createCliExchangeGrantForRequest() {
  return createApiClient().post<CliExchangeGrant>(ApiRouteTemplates.cliExchangeGrants, {
    auth: 'required',
  })
}

export async function listCliSessionsForRequest() {
  return createApiClient().get<CliSessions>(ApiRouteTemplates.cliSessions, {
    auth: 'required',
  })
}

export async function revokeCliSessionForRequest(data: RevokeCliSessionInput) {
  return createApiClient().delete<void>(
    buildApiPath(ApiRouteTemplates.cliSessionById, {
      session_id: data.sessionId,
    }),
    { auth: 'required' },
  )
}
