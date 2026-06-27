import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'

export function RouteErrorPage({
  error,
  fallbackMessage,
  title,
}: {
  error: unknown
  fallbackMessage: string
  title: string
}) {
  const message = error instanceof Error ? error.message : fallbackMessage

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] py-6">
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
    </main>
  )
}
