export class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message)
    this.name = 'HttpError'
  }
}

export async function loadJson<T>(
  url: RequestInfo | URL,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new HttpError(errorMessage(payload, response.status), response.status)
  }

  return payload as T
}

export function authHeaders(idToken?: string | null): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
}

export function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}

function errorMessage(payload: unknown, status: number) {
  if (
    payload &&
    typeof payload === 'object' &&
    'error' in payload &&
    typeof payload.error === 'string'
  ) {
    return payload.error
  }

  return `request failed: ${status}`
}
