import { ApiRouteTemplates, type ErrorResponse } from './types.generated'
import {
  apiValidators,
  type ApiValidationIssue,
  type ApiValidator,
} from './validators.generated'

export class HttpError extends Error {
  readonly errorReference: string | undefined

  constructor(
    readonly status: number,
    readonly response: ErrorResponse,
  ) {
    const errorReference = response.error_reference ?? undefined
    super(
      errorReference
        ? `${response.message} (reference: ${errorReference})`
        : response.message,
    )
    this.name = 'HttpError'
    this.errorReference = errorReference
  }
}

export type InvalidApiResponseFailure =
  | 'content-type'
  | 'json-syntax'
  | 'schema'
  | 'unexpected-content'
  | 'unexpected-no-content'

export class InvalidApiResponseError extends Error {
  constructor(
    readonly requestMethod: string,
    readonly requestPath: string,
    readonly status: number,
    readonly contentType: string | null,
    readonly failureClass: InvalidApiResponseFailure,
    readonly issuePath?: string,
  ) {
    const issue = issuePath ? `, issue ${issuePath}` : ''
    super(
      `${requestMethod} ${requestPath} returned an invalid API response ` +
        `(${failureClass}, ${status}, ${contentType ?? 'unknown content type'}${issue})`,
    )
    this.name = 'InvalidApiResponseError'
  }
}

type InvalidApiResponseObserver = (error: InvalidApiResponseError) => void
type NoContentValidator = ApiValidator<undefined> & { readonly noContent: true }

export const noContent: NoContentValidator = Object.assign(
  (value: unknown): value is undefined => value === undefined,
  { noContent: true as const },
)

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
  validator: ApiValidator<T>,
  init?: RequestInit,
  maxResponseBytes?: number,
): Promise<T> {
  const response = await fetch(url, init)
  if (!response.ok) await throwApiResponseError(response, url, init, maxResponseBytes)
  const context = responseContext(response, url, init)

  if (response.status === 204 || response.status === 205) {
    const payload: unknown = undefined
    if (validator(payload)) return payload
    throw invalidResponse(context, 'unexpected-no-content')
  }
  if (expectsNoContent(validator)) {
    throw invalidResponse(context, 'unexpected-content')
  }
  if (!isJsonContentType(context.contentType)) {
    throw invalidResponse(context, 'content-type')
  }

  let payload: unknown
  try {
    payload = maxResponseBytes === undefined
      ? await response.json()
      : JSON.parse(await readBoundedResponse(response, maxResponseBytes))
  } catch {
    throw invalidResponse(context, 'json-syntax')
  }

  if (!validator(payload)) {
    throw invalidResponse(
      context,
      'schema',
      validationIssuePath(validator.errors?.[0]),
    )
  }
  return payload
}

export async function throwApiResponseError(
  response: Response,
  url: RequestInfo | URL,
  init?: RequestInit,
  maxResponseBytes?: number,
): Promise<never> {
  const context = responseContext(response, url, init)
  if (!isJsonContentType(context.contentType)) {
    throw invalidResponse(context, 'content-type')
  }

  let payload: unknown
  try {
    payload = maxResponseBytes === undefined
      ? await response.json()
      : JSON.parse(await readBoundedResponse(response, maxResponseBytes))
  } catch {
    throw invalidResponse(context, 'json-syntax')
  }
  if (!apiValidators.ErrorResponse(payload)) {
    throw invalidResponse(
      context,
      'schema',
      validationIssuePath(apiValidators.ErrorResponse.errors?.[0]),
    )
  }
  throw new HttpError(response.status, payload)
}

export function arrayOf<T>(itemValidator: ApiValidator<T>): ApiValidator<T[]> {
  const validator: ApiValidator<T[]> = (value: unknown): value is T[] => {
    validator.errors = null
    if (!Array.isArray(value)) {
      validator.errors = [{ instancePath: '/', keyword: 'type' }]
      return false
    }
    for (const [index, item] of value.entries()) {
      if (itemValidator(item)) continue
      const issue = itemValidator.errors?.[0]
      validator.errors = [{
        instancePath: `/${index}${issue?.instancePath ?? ''}`,
        keyword: issue?.keyword ?? 'schema',
        message: issue?.message,
        params: issue?.params,
      }]
      return false
    }
    return true
  }
  validator.errors = null
  return validator
}

function expectsNoContent<T>(validator: ApiValidator<T>) {
  return 'noContent' in validator && validator.noContent === true
}

function invalidResponse(
  context: {
    contentType: string | null
    requestMethod: string
    requestPath: string
    status: number
  },
  failureClass: InvalidApiResponseFailure,
  issuePath?: string,
) {
  const error = new InvalidApiResponseError(
    context.requestMethod,
    context.requestPath,
    context.status,
    context.contentType,
    failureClass,
    issuePath,
  )
  observeInvalidApiResponse(error)
  return error
}

function responseContext(
  response: Response,
  url: RequestInfo | URL,
  init?: RequestInit,
) {
  return {
    contentType: response.headers.get('content-type'),
    requestMethod: requestMethod(url, init),
    requestPath: requestPath(url),
    status: response.status,
  }
}

export function validationIssuePath(issue: ApiValidationIssue | undefined) {
  if (!issue) return undefined
  const missing = issue.params?.missingProperty
  const path = missing
    ? `${issue.instancePath}/${String(missing).replaceAll('~', '~0').replaceAll('/', '~1')}`
    : issue.instancePath || '/'
  return path.slice(0, 160)
}

function isJsonContentType(contentType: string | null) {
  if (!contentType) return false
  const mediaType = contentType.split(';', 1)[0]?.trim().toLowerCase()
  return mediaType === 'application/json' || mediaType?.endsWith('+json') === true
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

export function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}

async function readBoundedResponse(response: Response, limit: number): Promise<string> {
  const reader = response.body?.getReader()
  if (!reader) return ''
  const decoder = new TextDecoder()
  let bytes = 0
  let text = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) return text + decoder.decode()
      bytes += value.byteLength
      if (bytes > limit) throw new Error('API response exceeds the byte limit.')
      text += decoder.decode(value, { stream: true })
    }
  } finally {
    await reader.cancel()
    reader.releaseLock()
  }
}
