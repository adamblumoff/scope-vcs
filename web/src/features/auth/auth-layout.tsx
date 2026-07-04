import { GitBranch, GlobeLock, History, ShieldCheck } from 'lucide-react'
import type { ReactNode } from 'react'

const highlights = [
  {
    icon: GlobeLock,
    title: 'Permissioned projections',
    description: 'Expose public history while keeping private files private.',
  },
  {
    icon: ShieldCheck,
    title: 'Config-owned visibility',
    description: 'Define audience rules in the repo config.',
  },
  {
    icon: History,
    title: 'CLI-owned pushes',
    description: 'Apply committed changes from the terminal.',
  },
]

export function AuthLayout({ children }: { children: ReactNode }) {
  return (
    <div className="grid min-h-dvh bg-background text-foreground lg:grid-cols-2">
      <aside className="relative hidden flex-col justify-between overflow-hidden border-r border-border bg-card p-10 lg:flex xl:p-14">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0"
          style={{
            backgroundImage:
              'radial-gradient(40rem 28rem at 15% 0%, var(--brand-muted), transparent 60%)',
          }}
        />
        <div className="relative flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-xl bg-brand text-brand-foreground shadow-[var(--shadow-card)]">
            <GitBranch className="size-5" />
          </div>
          <span className="text-lg font-semibold tracking-tight">Scope</span>
        </div>

        <div className="relative max-w-md">
          <h1 className="text-3xl font-semibold leading-tight tracking-tight xl:text-4xl">
            Source control with permission built in.
          </h1>
          <p className="mt-4 text-[15px] leading-6 text-muted-foreground">
            Scope projects a single repository into per-audience views, so you
            can share exactly what you mean to and nothing more.
          </p>

          <ul className="mt-10 space-y-5">
            {highlights.map((item) => (
              <li className="flex gap-3.5" key={item.title}>
                <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-background text-brand">
                  <item.icon className="size-4" />
                </div>
                <div>
                  <div className="text-sm font-medium leading-5">
                    {item.title}
                  </div>
                  <div className="mt-0.5 text-sm leading-5 text-muted-foreground">
                    {item.description}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </div>

        <div className="relative text-xs text-muted-foreground">
          Permissioned source-control projections.
        </div>
      </aside>

      <main className="flex items-center justify-center px-4 py-12 sm:px-6">
        <div className="w-full max-w-[400px]">
          <div className="mb-8 flex items-center gap-3 lg:hidden">
            <div className="flex size-9 items-center justify-center rounded-lg bg-brand text-brand-foreground shadow-[var(--shadow-card)]">
              <GitBranch className="size-4.5" />
            </div>
            <span className="text-base font-semibold tracking-tight">Scope</span>
          </div>
          <div className="flex justify-center lg:justify-start">{children}</div>
        </div>
      </main>
    </div>
  )
}
