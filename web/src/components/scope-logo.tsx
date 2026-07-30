import { cn } from '@/lib/utils'

export function ScopeLogo({ className }: { className?: string }) {
  return (
    <img
      alt="Scope"
      className={cn('block h-auto brightness-0 dark:brightness-100', className)}
      decoding="async"
      height={222}
      src="/brand/scope-wordmark.png"
      width={774}
    />
  )
}
