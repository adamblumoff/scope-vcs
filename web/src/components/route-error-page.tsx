import { PageErrorAlert } from '@/components/page-error-alert'
import { PageRail } from '@/components/page-header'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'

export function RouteErrorPage({
  error,
  fallbackMessage,
  title,
}: RouteErrorProps) {
  return (
    <main className="min-h-screen bg-background py-8 text-foreground">
      <RouteErrorContent
        className="py-6"
        error={error}
        fallbackMessage={fallbackMessage}
        title={title}
      />
    </main>
  )
}

export function RouteErrorContent({
  className,
  error,
  fallbackMessage,
  title,
}: RouteErrorProps) {
  const message = error instanceof Error ? error.message : fallbackMessage

  return (
    <PageRail className={cn('py-14', className)}>
      <div className="mx-auto max-w-[760px]">
        <PageErrorAlert className="mt-0" title={title}>
          {message}
        </PageErrorAlert>
        <Button asChild className="mt-5" size="sm" variant="secondary">
          <Link to="/">
            <ArrowLeft className="size-3.5" />
            <span>Home</span>
          </Link>
        </Button>
      </div>
    </PageRail>
  )
}

type RouteErrorProps = {
  className?: string
  error: unknown
  fallbackMessage: string
  title: string
}
