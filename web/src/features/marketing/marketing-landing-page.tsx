import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight, GitBranch } from 'lucide-react'
import type { ReactElement } from 'react'
import { RepositoryProjection } from './repository-projection'

const authParams = { _splat: '' }
const authRedirect = { redirect_url: '/' }
const marketingContainerClassName =
  'mx-auto w-[min(calc(100%-2rem),1280px)] sm:w-[min(calc(100%-3rem),1280px)]'

export function MarketingLandingPage(): ReactElement {
  return (
    <div className="dark marketing-page min-h-dvh text-foreground">
      <a
        className="fixed left-4 top-3 z-50 -translate-y-16 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background shadow-md focus:translate-y-0"
        href="#main-content"
      >
        Skip to content
      </a>

      <div className="grid min-h-dvh grid-rows-[66px_1fr] overflow-hidden sm:grid-rows-[74px_1fr]">
        <MarketingHeader />

        <main
          className={`${marketingContainerClassName} marketing-arena relative min-h-[1080px] py-10 outline-none sm:py-14`}
          id="main-content"
          tabIndex={-1}
        >
          <section className="marketing-copy relative z-10 max-w-[610px]">
            <h1 className="max-w-[680px] text-[clamp(2.6rem,13vw,3.2rem)] font-semibold leading-[0.95] tracking-[-0.067em] sm:text-[clamp(3.2rem,6.1vw,5.75rem)]">
              <span className="block whitespace-nowrap">Open source.</span>
              <span className="block whitespace-nowrap text-muted-foreground">On your terms.</span>
            </h1>

            <p className="mt-8 max-w-[540px] text-[clamp(1rem,1.4vw,1.1875rem)] leading-[1.62] tracking-[-0.015em] text-muted-foreground">
              Decide exactly what the public can see without splitting your repository or changing
              how your team works. Yes, you can safely commit and push your .env files—no
              third-party tooling required.
            </p>

            <div className="mt-9 flex flex-wrap items-center gap-3">
              <Button asChild className="h-12 gap-3.5 px-5 text-sm font-semibold">
                <Link params={authParams} search={authRedirect} to="/sign-up/$">
                  Try it out
                  <ArrowRight className="size-[17px] transition-transform group-hover/button:translate-x-0.5 motion-reduce:transform-none" />
                </Link>
              </Button>
            </div>
          </section>

          <RepositoryProjection />
        </main>
      </div>
    </div>
  )
}

function MarketingHeader(): ReactElement {
  return (
    <header
      className={`${marketingContainerClassName} flex items-center justify-between border-b border-border/80`}
    >
      <Link
        aria-label="Scope home"
        className="group flex items-center gap-3 rounded-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        to="/"
      >
        <span className="grid size-[34px] place-items-center rounded-full border border-border bg-[linear-gradient(180deg,#181b20,#111318)]">
          <GitBranch className="size-[18px] text-[var(--platinum-bright)]" strokeWidth={1.8} />
        </span>
        <span className="text-[17px] font-semibold tracking-[-0.025em]">Scope</span>
      </Link>

      <nav aria-label="Account">
        <Button asChild className="h-10 px-3 sm:px-4" variant="ghost">
          <Link params={authParams} search={authRedirect} to="/sign-in/$">
            Sign in
          </Link>
        </Button>
      </nav>
    </header>
  )
}
