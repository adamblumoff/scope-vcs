import { ApiRouteTemplates } from './types.generated'

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

type InvalidApiResponseObserver = (error: InvalidApiResponseError) => void

// Nitro can bundle the plugin and API client in separate chunks. This key keeps
// one process-wide observer shared by both copies of the module.
const invalidApiResponseObserverKey = Symbol.for(
  'scope.api.invalid-api-response-observer',
)

const apiRouteMatchers = Object.values(ApiRouteTemplates)
  .map((template) => {
    const segments = routeSegments(template)
    return {
      segments,
      staticSegments: segments.filter((segment) => !isRouteParameter(segment)).length,
      template,
    }
  })
  .sort((left, right) => right.staticSegments - left.staticSegments)

export function setInvalidApiResponseObserver(
  observer: InvalidApiResponseObserver | undefined,
) {
  invalidApiResponseObservers()[invalidApiResponseObserverKey] = observer
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
      const error = new InvalidApiResponseError(
        requestMethod(url, init),
        requestPath(url),
        response.status,
        response.headers.get('content-type'),
      )
      observeInvalidApiResponse(error)
      throw error
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

function observeInvalidApiResponse(error: InvalidApiResponseError) {
  try {
    invalidApiResponseObservers()[invalidApiResponseObserverKey]?.(error)
  } catch {
    // Observability must not replace the API error seen by the caller.
  }
}

function invalidApiResponseObservers() {
  return globalThis as unknown as Record<
    symbol,
    InvalidApiResponseObserver | undefined
  >
}

function requestMethod(url: RequestInfo | URL, init?: RequestInit) {
  return (init?.method ?? (url instanceof Request ? url.method : 'GET')).toUpperCase()
}

function requestPath(url: RequestInfo | URL) {
  const value = url instanceof Request ? url.url : String(url)
  try {
    const parsed = new URL(value, 'http://scope.invalid')
    return parsed.protocol === 'http:' || parsed.protocol === 'https:'
      ? apiRouteTemplate(parsed.pathname)
      : '[non-HTTP URL]'
  } catch {
    return '[invalid URL]'
  }
}

function apiRouteTemplate(pathname: string) {
  const segments = routeSegments(pathname)
  const matcher = apiRouteMatchers.find((candidate) =>
    candidate.segments.length === segments.length &&
    candidate.segments.every((segment, index) =>
      isRouteParameter(segment) || segment === segments[index],
    ))
  return matcher?.template ?? '[unrecognized API route]'
}

function routeSegments(path: string) {
  return path.split('/').filter(Boolean)
}

function isRouteParameter(segment: string) {
  return segment.startsWith('{') && segment.endsWith('}')
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
