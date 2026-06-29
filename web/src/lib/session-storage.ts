type SessionStorageReader = Pick<Storage, 'getItem' | 'removeItem'>
type SessionStorageWriter = SessionStorageReader & Pick<Storage, 'setItem'>

function browserSessionStorage(): SessionStorageWriter | null {
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
