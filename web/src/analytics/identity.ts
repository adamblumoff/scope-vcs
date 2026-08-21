export type IdentityTransition =
  | { kind: 'identify'; scopeUserId: string }
  | { kind: 'reset' }
  | { kind: 'none' }

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
