export { HttpError, authHeaders, loadJson, stripTrailingSlash } from './http'
import { stripTrailingSlash } from './http'

const localApiBase = 'http://localhost:8080'
const internalApiUrlEnv = 'SCOPE_API_INTERNAL_URL'
const publicApiUrlEnv = 'SCOPE_API_PUBLIC_URL'
export const homeFlashKey = 'scope:home-flash'

export async function readRequestAuthToken() {
  const { authCookieName } = await import('@/lib/auth')
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

export function getApiConnection(action = 'loading repositories') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getApiMutationConnection(action = 'changing repository state') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getPublicApiConnection(action = 'building repository setup') {
  return configuredApiConnection(publicApiUrlEnv, action)
}

function configuredApiConnection(envName: string, action: string) {
  const envBase = runtimeEnv(envName)
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error(`Set ${envName} before ${action}.`)
}

function runtimeEnv(name: string) {
  if (typeof process === 'undefined') {
    return undefined
  }

  return process.env[name]?.trim()
}

export function storeHomeFlash(message: string) {
  if (typeof window === 'undefined') {
    return
  }

  window.sessionStorage.setItem(homeFlashKey, message)
}

