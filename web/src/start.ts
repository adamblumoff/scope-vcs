import { forceSignedOut } from '@/auth-mode'
import { signedOutAuthMiddleware } from '@/server/signed-out-auth-middleware'
import { clerkMiddleware } from '@clerk/tanstack-react-start/server'
import { createStart } from '@tanstack/react-start'

export const startInstance = createStart(() => {
  return {
    requestMiddleware: [
      forceSignedOut ? signedOutAuthMiddleware : clerkMiddleware(),
    ],
  }
})
