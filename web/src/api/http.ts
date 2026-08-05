export class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly errorReference?: string,
  ) {
    super(errorReference ? `${message} (reference: ${errorReference})` : message)
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
    throw new HttpError(
      errorMessage(payload, response.status),
      response.status,
      errorReference(payload),
    )
  }

  return payload as T
}

function errorReference(payload: unknown) {
  if (
    payload &&
    typeof payload === 'object' &&
    'error_reference' in payload &&
    typeof payload.error_reference === 'string'
  ) {
    return payload.error_reference
  }

  return undefined
}

export function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}

function errorMessage(payload: unknown, status: number) {
  if (
    payload &&
    typeof payload === 'object' &&
    'message' in payload &&
    typeof payload.message === 'string'
  ) {
    return payload.message
  }

  return `request failed: ${status}`
}
