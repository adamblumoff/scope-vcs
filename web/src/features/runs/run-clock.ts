import { useSyncExternalStore } from 'react'

const TICK_MS = 15_000

const listeners = new Set<() => void>()
let timer: ReturnType<typeof setInterval> | null = null
let nowUnix = currentUnix()
let readAtMs = Date.now()

function currentUnix() {
  return Math.floor(Date.now() / 1_000)
}

function tick() {
  const next = currentUnix()
  if (next === nowUnix) return
  nowUnix = next
  readAtMs = Date.now()
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  timer ??= setInterval(tick, TICK_MS)
  return () => {
    listeners.delete(listener)
    if (listeners.size > 0 || timer === null) return
    clearInterval(timer)
    timer = null
  }
}

function snapshot() {
  return nowUnix
}

/**
 * Server renders have no ticking subscription, so refresh at most once a
 * second. Two reads inside one synchronous render return the same value.
 */
function serverSnapshot() {
  const elapsed = Date.now() - readAtMs
  if (elapsed >= 1_000) {
    nowUnix = currentUnix()
    readAtMs = Date.now()
  }
  return nowUnix
}

/**
 * One clock for every relative time and running duration on a runs page, so
 * they advance together instead of each surface owning a timer.
 */
export function useRunClock() {
  return useSyncExternalStore(subscribe, snapshot, serverSnapshot)
}
