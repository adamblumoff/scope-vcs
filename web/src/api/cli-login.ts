import { createApiClient } from '@/api/client'
import type { CompleteCliLoginInput } from './cli-login-input'
import type { DeviceLoginCompleteResponse } from './types.generated'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  return createApiClient().post<DeviceLoginCompleteResponse>(
    `/v1/cli/device-login/${encodeURIComponent(data.code)}/complete`,
    { auth: 'required' },
  )
}
