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
  CliAuthApiEndpoints,
  type DeviceLoginCompleteResponse,
} from './types.generated'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  return createApiClient().post<DeviceLoginCompleteResponse>(
    `${CliAuthApiEndpoints.deviceLoginStart}/${encodeURIComponent(data.code)}/complete`,
    { auth: 'required' },
  )
}

export async function completeBrowserCliLoginForRequest(
  data: CompleteBrowserCliLoginInput,
) {
  return createApiClient().post<BrowserLoginComplete>(
    `${CliAuthApiEndpoints.browserLoginStart}/${encodeURIComponent(data.requestId)}/complete`,
    { auth: 'required' },
  )
}

export async function createCliExchangeGrantForRequest() {
  return createApiClient().post<CliExchangeGrant>('/v1/cli/exchange-grants', {
    auth: 'required',
  })
}

export async function listCliSessionsForRequest() {
  return createApiClient().get<CliSessions>('/v1/cli/sessions', {
    auth: 'required',
  })
}

export async function revokeCliSessionForRequest(data: RevokeCliSessionInput) {
  return createApiClient().delete<void>(
    `/v1/cli/sessions/${encodeURIComponent(data.sessionId)}`,
    { auth: 'required' },
  )
}
