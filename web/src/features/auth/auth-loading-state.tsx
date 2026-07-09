import { Skeleton } from '@/components/ui/skeleton'
import type { ReactNode } from 'react'

export function AuthSurface({
  children,
  description,
  title,
}: {
  children: ReactNode
  description: string
  title: string
}) {
  return (
    <section className="w-full">
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-1.5 text-sm leading-5 text-muted-foreground">
        {description}
      </p>
      <div className="mt-5">{children}</div>
    </section>
  )
}

export function AuthLoadingState({ label }: { label: string }) {
  return (
    <output
      aria-label={label}
      className="block w-full max-w-sm border-y border-border py-6"
    >
      <div className="text-sm font-medium">{label}</div>
      <div className="mt-1 text-sm text-muted-foreground">
        Connecting to secure account services…
      </div>
      <div aria-hidden className="mt-6 space-y-3">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-9 w-28" />
      </div>
    </output>
  )
}
