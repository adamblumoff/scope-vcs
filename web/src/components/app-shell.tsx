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
    <div className="flex h-dvh min-h-0 flex-col overflow-hidden bg-background text-foreground">
      <a
        className="fixed left-4 top-3 z-50 -translate-y-16 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background shadow-md focus:translate-y-0"
        href="#main-content"
      >
        Skip to content
      </a>
      {header?.()}
      {subheader?.()}
      <main
        className={cn(
          'min-h-0 flex-1 overflow-y-auto overscroll-contain outline-none',
          className,
        )}
        id="main-content"
        tabIndex={-1}
      >
        {children}
      </main>
    </div>
  )
}
