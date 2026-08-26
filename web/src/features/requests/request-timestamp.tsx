import { useHydrated } from '@/lib/use-hydrated'
import { useUnixClock } from '@/lib/use-unix-clock'
import {
  formatRelativeUnix,
  formatUnixDate,
  formatUnixDateUtc,
} from './request-labels'

/** Relative request time with the reader's absolute local time on hover. */
export function RequestTimestamp({
  className,
  value,
}: {
  className?: string
  value: number
}) {
  const hydrated = useHydrated()
  const nowUnix = useUnixClock()
  const date = new Date(value * 1_000)

  return (
    <time
      className={className}
      dateTime={date.toISOString()}
      suppressHydrationWarning
      title={hydrated ? formatUnixDate(value) : undefined}
    >
      {formatRelativeUnix(value, nowUnix)}
    </time>
  )
}

/**
 * Absolute request time that switches from deterministic UTC to browser local.
 */
export function RequestAbsoluteTimestamp({
  className,
  prefix = '',
  value,
}: {
  className?: string
  prefix?: string
  value: number | null
}) {
  const hydrated = useHydrated()
  if (value === null) {
    return (
      <span className={className}>
        {prefix}
        {formatUnixDate(null)}
      </span>
    )
  }
  const date = new Date(value * 1_000)

  return (
    <time
      className={className}
      dateTime={date.toISOString()}
      suppressHydrationWarning
    >
      {prefix}
      {hydrated ? formatUnixDate(value) : formatUnixDateUtc(value)}
    </time>
  )
}
