export type SessionStorageReader = Pick<Storage, 'getItem' | 'removeItem'>
export type SessionStorageWriter = SessionStorageReader &
  Pick<Storage, 'setItem'>

export function browserSessionStorage(): SessionStorageWriter | null {
  if (typeof window === 'undefined') {
    return null
  }

  return window.sessionStorage
}

export function readAndClearSessionValue(
  key: string,
  storage: SessionStorageReader | null = browserSessionStorage(),
) {
  if (!storage) {
    return null
  }

  const value = storage.getItem(key)
  if (value !== null) {
    storage.removeItem(key)
  }
  return value
}

export function storeSessionValue(
  key: string,
  value: string,
  storage: SessionStorageWriter | null = browserSessionStorage(),
) {
  storage?.setItem(key, value)
}

export function clearSessionValue(
  key: string,
  storage: Pick<Storage, 'removeItem'> | null = browserSessionStorage(),
) {
  storage?.removeItem(key)
}
