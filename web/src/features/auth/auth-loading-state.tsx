import { PendingSurface } from '@/components/pending-surface'
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
    <PendingSurface
      className="min-h-[220px] w-full max-w-sm"
      label={label}
    />
  )
}
