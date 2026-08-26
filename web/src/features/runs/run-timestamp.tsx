import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
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
  const nowUnix = useUnixClock()
  const hydrated = useHydrated()
  const date = runUnixTimeDate(value)

  return (
    <time
      dateTime={date.toISOString()}
      suppressHydrationWarning
      // The absolute time is only meaningful in the reader's own zone, and
      // suppressed hydration does not repaint a title rendered server-side.
      title={hydrated ? ABSOLUTE_FORMATTER.format(date) : undefined}
    >
      {formatRelativeTime(value, nowUnix)}
    </time>
  )
}
