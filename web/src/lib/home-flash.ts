import { useState, useSyncExternalStore } from 'react'
import {
  readAndClearSessionValue,
  storeSessionValue,
} from './session-storage'

const homeFlashKey = 'scope:home-flash'

type HomeFlashSnapshot = {
  value: string | null | undefined
}

export function storeHomeFlash(message: string) {
  storeSessionValue(homeFlashKey, message)
}

export function useHomeFlash() {
  const [snapshot] = useState<HomeFlashSnapshot>(() => ({ value: undefined }))
  return useSyncExternalStore(
    subscribeHomeFlash,
    () => getHomeFlashSnapshot(snapshot),
    getServerHomeFlashSnapshot,
  )
}

function subscribeHomeFlash() {
  return () => {}
}

function getHomeFlashSnapshot(snapshot: HomeFlashSnapshot) {
  if (snapshot.value !== undefined) {
    return snapshot.value
  }

  snapshot.value = readAndClearSessionValue(homeFlashKey)
  return snapshot.value
}

function getServerHomeFlashSnapshot() {
  return null
}
