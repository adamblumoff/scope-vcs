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

export class InvalidApiResponseError extends Error {
  constructor(
    readonly requestMethod: string,
    readonly requestPath: string,
    readonly status: number,
    readonly contentType: string | null,
  ) {
    super(
      `${requestMethod} ${requestPath} returned invalid JSON ` +
        `(${status}, ${contentType ?? 'unknown content type'})`,
    )
    this.name = 'InvalidApiResponseError'
  }
}

export async function loadJson<T>(
  url: RequestInfo | URL,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(url, init)
  if (response.status === 204 || response.status === 205) {
    return undefined as T
  }

  let payload: unknown
  try {
    payload = await response.json()
  } catch {
    if (!response.ok) {
      payload = null
    } else {
      throw new InvalidApiResponseError(
        requestMethod(url, init),
        requestPath(url),
        response.status,
        response.headers.get('content-type'),
      )
    }
  }

  if (!response.ok) {
    throw new HttpError(
      errorMessage(payload, response.status),
      response.status,
      errorReference(payload),
    )
  }

  return payload as T
}

function requestMethod(url: RequestInfo | URL, init?: RequestInit) {
  return (init?.method ?? (url instanceof Request ? url.method : 'GET')).toUpperCase()
}

function requestPath(url: RequestInfo | URL) {
  const value = url instanceof Request ? url.url : String(url)
  try {
    const parsed = new URL(value, 'http://scope.invalid')
    return parsed.protocol === 'http:' || parsed.protocol === 'https:'
      ? parsed.pathname
      : '[non-HTTP URL]'
  } catch {
    return '[invalid URL]'
  }
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
