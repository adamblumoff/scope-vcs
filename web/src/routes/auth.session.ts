import {
  authCookieMaxAgeSeconds,
  authCookieName,
  authCookiePath,
  isSameOriginRequest,
  isSecureRequest,
} from '@/lib/auth'
import { createFileRoute } from '@tanstack/react-router'

type SessionRequestBody = {
  idToken?: unknown
  expiresIn?: unknown
}

export const Route = createFileRoute('/auth/session')({
  server: {
    handlers: {
      POST: async ({ request }) => {
        if (!isSameOriginRequest(request)) {
          return json({ error: 'cross-origin auth session request rejected' }, 403)
        }

        const body = (await request.json().catch(() => null)) as
          | SessionRequestBody
          | null
        const idToken = typeof body?.idToken === 'string' ? body.idToken.trim() : ''

        if (!idToken) {
          return json({ error: 'missing Shoo token' }, 400)
        }

        const { setCookie } = await import('@tanstack/react-start/server')
        setCookie(authCookieName, idToken, {
          httpOnly: true,
          maxAge: authCookieMaxAgeSeconds(body?.expiresIn),
          path: authCookiePath,
          sameSite: 'lax',
          secure: isSecureRequest(request),
        })

        return json({ ok: true })
      },
      DELETE: async ({ request }) => {
        if (!isSameOriginRequest(request)) {
          return json({ error: 'cross-origin auth session request rejected' }, 403)
        }

        const { deleteCookie } = await import('@tanstack/react-start/server')
        deleteCookie(authCookieName, {
          path: authCookiePath,
          secure: isSecureRequest(request),
        })

        return json({ ok: true })
      },
    },
  },
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    headers: {
      'cache-control': 'no-store',
      'content-type': 'application/json',
    },
    status,
  })
}
