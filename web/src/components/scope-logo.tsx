import { cn } from '@/lib/utils'

export function ScopeLogo({ className }: { className?: string }) {
  return (
    <img
      alt="Scope"
      className={cn('block h-auto brightness-0 dark:invert', className)}
      decoding="async"
      height={96}
      src="/brand/scope-lockup.svg"
      width={382}
    />
  )
}

export function ScopeMark({ className }: { className?: string }) {
  return (
    <img
      alt=""
      aria-hidden
      className={cn('block h-auto brightness-0 dark:invert', className)}
      decoding="async"
      height={96}
      src="/brand/scope-mark.svg"
      width={96}
    />
  )
}
