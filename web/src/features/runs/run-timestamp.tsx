import { useRunClock } from './run-clock'
import {
  createRunTimeFormatter,
  formatRelativeTime,
  runUnixTimeDate,
} from './run-formatting'

const ABSOLUTE_FORMATTER = createRunTimeFormatter()

/**
 * Relative text for scanning, with the absolute time one hover away. The
 * relative value depends on the reader's clock, so hydration is allowed to
 * settle it rather than matching the server byte for byte.
 */
export function RunTimestamp({ value }: { value: number }) {
  const nowUnix = useRunClock()
  const date = runUnixTimeDate(value)

  return (
    <time
      dateTime={date.toISOString()}
      suppressHydrationWarning
      title={ABSOLUTE_FORMATTER.format(date)}
    >
      {formatRelativeTime(value, nowUnix)}
    </time>
  )
}
