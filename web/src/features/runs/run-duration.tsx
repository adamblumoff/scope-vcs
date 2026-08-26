import { useUnixClock } from '@/lib/use-unix-clock'
import { elapsedDuration } from './run-formatting'

/**
 * How long a run, job, attempt or step took. While it is still going the value
 * counts up from the shared clock.
 */
export function RunDuration({
  end,
  start,
}: {
  end: number | null
  start: number | null
}) {
  const nowUnix = useUnixClock()
  const value = elapsedDuration(start, end, nowUnix)

  return (
    <span className="tabular-nums" suppressHydrationWarning>
      {value ?? '—'}
    </span>
  )
}
