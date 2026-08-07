import { signedOutPublishableKey } from '@/auth-mode'
import { createMiddleware } from '@tanstack/react-start'

const signedOutAuth = {
  actor: null,
  debug: () => ({}),
  factorVerificationAge: null,
  getToken: () => Promise.resolve(null),
  has: () => false,
  isAuthenticated: false,
  orgId: null,
  orgPermissions: null,
  orgRole: null,
  orgSlug: null,
  sessionClaims: null,
  sessionId: null,
  sessionStatus: null,
  tokenType: 'session_token',
  userId: null,
} as const

export const signedOutAuthMiddleware = createMiddleware().server(
  ({ next }) =>
    next({
      context: {
        auth: () => signedOutAuth,
        clerkInitialState: {
          __internal_clerk_state: {
            __publishableKey: signedOutPublishableKey,
            __clerk_ssr_state: {
              actor: null,
              factorVerificationAge: null,
              orgId: null,
              orgPermissions: null,
              orgRole: null,
              orgSlug: null,
              organization: null,
              session: null,
              sessionClaims: null,
              sessionId: null,
              sessionStatus: null,
              user: null,
              userId: null,
            },
          },
        },
      },
    }),
)
