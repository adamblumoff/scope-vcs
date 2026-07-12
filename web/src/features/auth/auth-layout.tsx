import { GitBranch } from 'lucide-react'
import type { ReactNode } from 'react'

export function AuthLayout({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-dvh bg-background text-foreground">
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex min-h-16 max-w-[1200px] items-center gap-2.5 px-4 sm:px-6 lg:px-8">
          <GitBranch className="size-[18px] text-[var(--platinum-bright)]" strokeWidth={1.8} />
          <span className="text-[17px] font-semibold tracking-[-0.025em]">Scope</span>
        </div>
      </header>
      <main className="mx-auto grid min-h-[calc(100dvh-65px)] max-w-[1200px] items-center gap-12 px-4 py-12 sm:px-6 lg:grid-cols-[minmax(0,1fr)_400px] lg:px-8">
        <div className="hidden max-w-xl lg:block">
          <div className="font-mono text-[10px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            Permissioned source control
          </div>
          <h1 className="mt-4 text-4xl font-semibold leading-[1.08] tracking-[-0.045em] text-foreground xl:text-5xl">
            Share exactly what you mean to.
          </h1>
          <p className="mt-5 max-w-lg text-[15px] leading-7 text-muted-foreground">
            Scope projects one repository into audience-specific views while ordinary Git remains the workflow.
          </p>
          <div className="mt-9 grid grid-cols-3 border-y border-border py-5 text-xs text-muted-foreground">
            <span>Config-owned visibility</span>
            <span className="border-l border-border pl-5">Projected history</span>
            <span className="border-l border-border pl-5">CLI-first changes</span>
          </div>
        </div>
        <div className="w-full max-w-[400px] justify-self-center lg:justify-self-end">
          {children}
        </div>
      </main>
    </div>
  )
}
