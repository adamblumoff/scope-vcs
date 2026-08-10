import { useSyncExternalStore } from 'react'
import { createRunTimeFormatter, runUnixTimeDate } from './run-formatting'

const BROWSER_LOCAL_FORMATTER = createRunTimeFormatter()
const UTC_HYDRATION_FORMATTER = createRunTimeFormatter('UTC')

export function RunTimestamp({ value }: { value: number }) {
  const hydrated = useSyncExternalStore(
    subscribeToHydration,
    getBrowserSnapshot,
    getServerSnapshot,
  )
  const date = runUnixTimeDate(value)
  const formatter = hydrated
    ? BROWSER_LOCAL_FORMATTER
    : UTC_HYDRATION_FORMATTER

  return (
    <time dateTime={date.toISOString()}>
      {formatter.format(date)}
    </time>
  )
}

function subscribeToHydration() {
  return () => {}
}

function getBrowserSnapshot() {
  return true
}

function getServerSnapshot() {
  return false
}
