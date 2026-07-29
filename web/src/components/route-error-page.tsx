import { PageErrorAlert } from '@/components/page-error-alert'
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
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <RouteErrorContent
        className="px-0 py-6 sm:px-0"
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
    <div
      className={cn(
        'mx-auto max-w-[760px] px-4 py-14 sm:px-6',
        className,
      )}
    >
      <PageErrorAlert className="mt-0" title={title}>
        {message}
      </PageErrorAlert>
      <Button asChild className="mt-5" size="sm" variant="secondary">
        <Link to="/">
          <ArrowLeft className="size-3.5" />
          <span>Repos</span>
        </Link>
      </Button>
    </div>
  )
}

type RouteErrorProps = {
  className?: string
  error: unknown
  fallbackMessage: string
  title: string
}
