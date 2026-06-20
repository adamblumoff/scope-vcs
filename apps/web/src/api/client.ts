import { authCookieName } from '@/lib/auth'

const localApiBase = 'http://localhost:8080'
export const homeFlashKey = 'scope:home-flash'

export class HttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message)
  }
}

export async function readRequestAuthToken() {
  const { getCookie } = await import('@tanstack/react-start/server')
  return getCookie(authCookieName)
}

export async function loadJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init)
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new HttpError(
      payload?.error ?? `request failed: ${response.status}`,
      response.status,
    )
  }

  return payload as T
}

export function authHeaders(idToken?: string): HeadersInit {
  return idToken ? { authorization: `Bearer ${idToken}` } : {}
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

export function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
