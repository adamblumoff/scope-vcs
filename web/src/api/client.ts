export {
  arrayOf,
  HttpError,
  InvalidApiResponseError,
  loadJson,
  noContent,
  stripTrailingSlash,
} from './http'
import { loadJson, stripTrailingSlash } from './http'
import type { ApiValidator } from './validators.generated'

const localApiBase = 'http://localhost:8080'
const localCliInstallBase = 'http://localhost:8787'
const cliInstallUrlEnv = 'SCOPE_CLI_INSTALL_URL'
const internalApiUrlEnv = 'SCOPE_API_INTERNAL_URL'
const publicApiUrlEnv = 'SCOPE_API_PUBLIC_URL'
const clerkApiTokenTemplateEnv = 'SCOPE_CLERK_API_TOKEN_TEMPLATE'
const nodeEnv = 'NODE_ENV'
const defaultClerkApiTokenTemplate = 'scope_api'

export type ApiAuthMode = 'none' | 'optional' | 'required'

type ApiRequestOptions = {
  auth?: ApiAuthMode
  body?: unknown
  signal?: AbortSignal
}

export type ApiClient = ReturnType<typeof createApiClient>

export function createApiClient() {
  let tokenPromise: Promise<string | null> | undefined

  async function request<T>(
    method: string,
    path: string,
    validator: ApiValidator<T>,
    options: ApiRequestOptions = {},
  ): Promise<T> {
    const headers = new Headers()
    if (options.body !== undefined) {
      headers.set('content-type', 'application/json')
    }

    const authMode = options.auth ?? 'none'
    if (authMode !== 'none') {
      const token = await requestAuthToken()
      if (!token && authMode === 'required') {
        throw new Error('Sign in required.')
      }
      if (token) {
        headers.set('authorization', `Bearer ${token}`)
      }
    }

    return loadJson(`${connectionForMethod(method)}${path}`, validator, {
      body:
        options.body === undefined ? undefined : JSON.stringify(options.body),
      headers,
      method,
      signal: options.signal,
    })
  }

  async function requestAuthToken() {
    tokenPromise ??= readClerkApiToken()
    return tokenPromise
  }

  return {
    authenticated: async () => Boolean(await requestAuthToken()),
    delete: <T>(path: string, validator: ApiValidator<T>, options?: ApiRequestOptions) =>
      request('DELETE', path, validator, options),
    get: <T>(path: string, validator: ApiValidator<T>, options?: ApiRequestOptions) =>
      request('GET', path, validator, options),
    patch: <T>(path: string, validator: ApiValidator<T>, options?: ApiRequestOptions) =>
      request('PATCH', path, validator, options),
    post: <T>(path: string, validator: ApiValidator<T>, options?: ApiRequestOptions) =>
      request('POST', path, validator, options),
    put: <T>(path: string, validator: ApiValidator<T>, options?: ApiRequestOptions) =>
      request('PUT', path, validator, options),
  }
}

async function readClerkApiToken() {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { getToken, isAuthenticated } = await auth()
  if (!isAuthenticated) {
    return null
  }
  return getToken({ template: clerkApiTokenTemplate() })
}

export function clerkApiTokenTemplate() {
  return runtimeEnv(clerkApiTokenTemplateEnv) ?? defaultClerkApiTokenTemplate
}

function connectionForMethod(method: string) {
  return method === 'GET' ? getApiConnection() : getApiMutationConnection()
}

export function getApiConnection(action = 'loading repositories') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getApiMutationConnection(action = 'changing repository state') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getPublicApiConnection(action = 'building public API URL') {
  return configuredApiConnection(publicApiUrlEnv, action)
}

export function getCliInstallConnection(action = 'building CLI install command') {
  return configuredConnection(cliInstallUrlEnv, localCliInstallBase, action)
}

function configuredApiConnection(envName: string, action: string) {
  return configuredConnection(envName, localApiBase, action)
}

function configuredConnection(envName: string, fallback: string, action: string) {
  const envBase = runtimeEnv(envName)
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (runtimeEnv(nodeEnv) === 'development') {
    return fallback
  }

  throw new Error(`Set ${envName} before ${action}.`)
}

function runtimeEnv(name: string) {
  if (typeof process === 'undefined') {
    return undefined
  }

  return process.env[name]?.trim()
}
