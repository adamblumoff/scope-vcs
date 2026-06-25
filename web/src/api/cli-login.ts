import {
  authHeaders,
  getApiMutationConnection,
  readRequestAuthToken,
} from '@/api/client'
import type { CompleteCliLoginInput } from './cli-login-input'

export async function completeCliLoginForRequest(
  data: CompleteCliLoginInput,
) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in to authorize the CLI.')
  }

  const response = await fetch(
    `${getApiMutationConnection('authorizing CLI login')}/v1/cli/device-login/${encodeURIComponent(
      data.code,
    )}/complete`,
    {
      headers: authHeaders(idToken),
      method: 'POST',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload
}
