export { HttpError, authHeaders, loadJson, stripTrailingSlash } from './http'
import { stripTrailingSlash } from './http'

const localApiBase = 'http://localhost:8080'
export const homeFlashKey = 'scope:home-flash'

export async function readRequestAuthToken() {
  const { authCookieName } = await import('@/lib/auth')
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

export function getApiConnection(action = 'loading repositories') {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error(`Set VITE_SCOPE_API_URL before ${action}.`)
}

export function getApiMutationConnection(action = 'changing repository state') {
  const envBase = import.meta.env.VITE_SCOPE_API_URL as string | undefined
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return localApiBase
  }

  throw new Error(`Set VITE_SCOPE_API_URL before ${action}.`)
}

export function storeHomeFlash(message: string) {
  if (typeof window === 'undefined') {
    return
  }

  window.sessionStorage.setItem(homeFlashKey, message)
}

