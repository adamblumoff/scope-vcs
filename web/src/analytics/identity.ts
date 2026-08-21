export type IdentityTransition =
  | { kind: 'identify'; scopeUserId: string }
  | { kind: 'reset' }
  | { kind: 'none' }

export type ResolvedAnalyticsIdentity = {
  identityKey: string
  scopeUserId: string | null
}

export async function resolveAnalyticsIdentity(
  clerkUserId: string,
  loadIdentity: () => Promise<{ scopeUserId: string } | null>,
): Promise<ResolvedAnalyticsIdentity> {
  try {
    const identity = await loadIdentity()
    return {
      identityKey: identifiedKey(clerkUserId),
      scopeUserId: identity?.scopeUserId ?? null,
    }
  } catch {
    return {
      identityKey: identifiedKey(clerkUserId),
      scopeUserId: null,
    }
  }
}

export function identityTransition(input: {
  currentDistinctId: string
  isSignedIn: boolean
  persistedUserId: unknown
  scopeUserId?: string | null
}): IdentityTransition {
  if (!input.isSignedIn) {
    const hasScopeIdentity = input.currentDistinctId.startsWith('scope_usr_')
      || Boolean(input.persistedUserId)
    return hasScopeIdentity ? { kind: 'reset' } : { kind: 'none' }
  }

  if (
    input.scopeUserId
    && input.currentDistinctId !== input.scopeUserId
  ) {
    return { kind: 'identify', scopeUserId: input.scopeUserId }
  }

  return { kind: 'none' }
}

function identifiedKey(clerkUserId: string) {
  return `identified:${clerkUserId}`
}
