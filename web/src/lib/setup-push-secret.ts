import {
  browserSessionStorage,
  clearSessionValue,
  readAndClearSessionValue,
  storeSessionValue,
} from './session-storage'
import { setupPushSecretKey } from '../api/setup-token-key'

type SetupPushSecretReader = Pick<Storage, 'getItem' | 'removeItem'>
type SetupPushSecretWriter = SetupPushSecretReader & Pick<Storage, 'setItem'>

const setupPushSecretsByRepo = new Map<string, string | null>()

export function rememberSetupPushSecret(
  repoId: string,
  secret: string | null,
) {
  setupPushSecretsByRepo.set(repoId, secret)
}

export function storeSetupPushSecret(
  repoId: string,
  secret: string | null,
  storage: SetupPushSecretWriter | null = browserSessionStorage(),
) {
  rememberSetupPushSecret(repoId, secret)
  const key = setupPushSecretKey(repoId)
  if (secret) {
    storeSessionValue(key, secret, storage)
  } else {
    clearSessionValue(key, storage)
  }
}

export function setupPushSecretSnapshot(
  repoId: string,
  storage: SetupPushSecretReader | null = browserSessionStorage(),
) {
  const key = setupPushSecretKey(repoId)
  if (setupPushSecretsByRepo.has(repoId)) {
    clearSessionValue(key, storage)
    return setupPushSecretsByRepo.get(repoId) ?? null
  }

  const secret = readAndClearSessionValue(key, storage)
  rememberSetupPushSecret(repoId, secret)
  return secret
}
