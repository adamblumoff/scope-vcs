import { createShooAuth } from '@shoojs/auth'
import type { ShooAuthOptions } from '@shoojs/auth'

export const authCookieName = 'scope_session'
export const authCookiePath = '/'

export const shooAuthOptions = {
  shooBaseUrl: 'https://shoo.dev',
  callbackPath: '/auth/callback',
  requestPii: true,
} satisfies ShooAuthOptions

export function createScopeShooAuth() {
  return createShooAuth(shooAuthOptions)
}

export function authCookieMaxAgeSeconds(expiresIn: unknown) {
  const fallback = 60 * 60
  const max = 24 * 60 * 60
  const parsed = typeof expiresIn === 'number' ? Math.floor(expiresIn) : fallback

  if (!Number.isFinite(parsed) || parsed <= 0) {
    return fallback
  }

  return Math.min(parsed, max)
}

export function isSecureRequest(request: Request) {
  const forwardedProto = request.headers
    .get('x-forwarded-proto')
    ?.split(',')[0]
    ?.trim()

  return forwardedProto === 'https' || new URL(request.url).protocol === 'https:'
}

export function isSameOriginRequest(request: Request) {
  const fetchSite = request.headers.get('sec-fetch-site')
  if (fetchSite === 'cross-site') {
    return false
  }

  const origin = request.headers.get('origin')
  if (!origin) {
    return true
  }

  return origin === new URL(request.url).origin
}
