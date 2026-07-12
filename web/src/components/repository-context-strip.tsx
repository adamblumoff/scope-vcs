import type { ReactNode } from 'react'

export type ContextFact = {
  id: string
  label: ReactNode
  semantic?: 'danger' | 'info' | 'success' | 'warning'
}

export function RepositoryContextStrip({ facts }: { facts: ContextFact[] }) {
  if (facts.length === 0) return null

  return (
    <div className="border-b border-border bg-background">
      <div className="mx-auto flex min-h-12 max-w-[1440px] items-center gap-0 overflow-x-auto px-4 text-xs text-muted-foreground sm:px-6 lg:px-8">
        {facts.map((fact) => (
          <div
            className="flex shrink-0 items-center gap-2 border-border px-4 first:pl-0 [&:not(:first-child)]:border-l"
            key={fact.id}
          >
            {fact.semantic && (
              <span
                aria-hidden
                className="size-1.5 rounded-full"
                style={{ background: `var(--${fact.semantic})` }}
              />
            )}
            <span>{fact.label}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
