import { setupPushSecretKey } from '../../api/setup-token-key'

type SetupPushSecretStorage = Pick<Storage, 'getItem' | 'removeItem'>

const setupPushSecretsByRepo = new Map<string, string | null>()

export function rememberSetupPushSecret(
  repoId: string,
  secret: string | null,
) {
  setupPushSecretsByRepo.set(repoId, secret)
}

export function setupPushSecretSnapshot(
  repoId: string,
  storage = browserSessionStorage(),
) {
  if (setupPushSecretsByRepo.has(repoId)) {
    return setupPushSecretsByRepo.get(repoId) ?? null
  }

  if (!storage) {
    return null
  }

  const key = setupPushSecretKey(repoId)
  const secret = storage.getItem(key)
  if (secret) {
    storage.removeItem(key)
  }
  rememberSetupPushSecret(repoId, secret)
  return secret
}

function browserSessionStorage(): SetupPushSecretStorage | null {
  if (typeof window === 'undefined') {
    return null
  }

  return window.sessionStorage
}
