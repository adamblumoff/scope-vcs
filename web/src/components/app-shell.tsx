import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

export function AppShell({
  children,
  className,
  header,
  subheader,
}: {
  children: ReactNode
  className?: string
  header?: () => ReactNode
  subheader?: () => ReactNode
}) {
  return (
    <div className="min-h-dvh bg-background text-foreground">
      <a
        className="fixed left-4 top-3 z-50 -translate-y-16 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background shadow-md focus:translate-y-0"
        href="#main-content"
      >
        Skip to content
      </a>
      {header?.()}
      {subheader?.()}
      <main
        className={cn('outline-none', className)}
        id="main-content"
        tabIndex={-1}
      >
        {children}
      </main>
    </div>
  )
}
