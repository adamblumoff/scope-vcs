import { PageRail } from '@/components/page-header'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'

export function RouteNotFoundPage() {
  return (
    <main className="min-h-screen bg-background py-8 text-foreground">
      <PageRail className="py-20 sm:py-28">
        <div className="max-w-xl">
          <p className="font-mono text-xs tracking-[0.18em] text-muted-foreground uppercase">
            404 · Not found
          </p>
          <h1 className="mt-3 text-3xl font-semibold tracking-tight">
            Nothing lives at this address.
          </h1>
          <p className="mt-3 text-sm leading-6 text-muted-foreground">
            The repository or page may have moved, or you may not have access.
          </p>
          <Button asChild className="mt-7" size="sm" variant="secondary">
            <Link to="/">
              <ArrowLeft className="size-3.5" />
              <span>Home</span>
            </Link>
          </Button>
        </div>
      </PageRail>
    </main>
  )
}
