import type { RepoDetail, RepoSummary } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { RepoStatusBadge } from '@/components/repo-status-badge'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Link } from '@tanstack/react-router'
import { AlertCircle, ArrowLeft, ArrowRight, FileSearch } from 'lucide-react'

export function RepoDetailPage({ detail }: { detail: RepoDetail }) {
  const { files, repo } = detail

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={repo.id} subtitleClassName="font-mono" />

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <RepoStatusBadge state={repo.lifecycle_state} />
              {repo.role === 'Owner' && (
                <VisibilityBadge visibility={repo.default_visibility} />
              )}
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
              )}
            </div>
            <h1 className="truncate font-mono text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              {repo.id}
            </h1>
          </div>
          <RepoAction repo={repo} />
        </div>

        <section className="mt-8 border-y border-border">
          {files.length === 0 ? (
            <div className="flex items-center gap-3 py-10">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
                <FileSearch className="size-5 text-muted-foreground" />
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium leading-5">No live files</div>
                <div className="mt-1 text-sm leading-5 text-muted-foreground">
                  {repo.lifecycle_state === 'PendingPublish'
                    ? 'Review the pending import before publishing.'
                    : 'Files will appear here after the repo has published content.'}
                </div>
              </div>
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>File</TableHead>
                  <TableHead className="w-[120px]">Visibility</TableHead>
                  <TableHead className="w-[90px]">Git</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {files.map((file) => (
                  <TableRow key={file.path}>
                    <TableCell className="max-w-[460px] truncate font-mono text-xs sm:max-w-[700px]">
                      {file.path}
                    </TableCell>
                    <TableCell>
                      <VisibilityBadge visibility={file.visibility} />
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">
                        {file.tracked ? 'Tracked' : 'Untracked'}
                      </Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </section>
      </section>
    </main>
  )
}

function RepoAction({ repo }: { repo: RepoSummary }) {
  if (repo.role !== 'Owner') {
    return null
  }

  if (repo.lifecycle_state === 'PendingFirstPush') {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/setup"
        >
          <ArrowRight className="size-3.5" />
          <span>Setup</span>
        </Link>
      </Button>
    )
  }

  if (repo.lifecycle_state === 'PendingPublish' || repo.staged_update_pending) {
    return (
      <Button asChild size="sm">
        <Link
          params={{ owner: repo.owner_handle, repo: repo.name }}
          to="/repos/$owner/$repo/review"
        >
          <ArrowRight className="size-3.5" />
          <span>Review</span>
        </Link>
      </Button>
    )
  }

  return null
}

export function RepoDetailError({ error }: { error: unknown }) {
  const message =
    error instanceof Error ? error.message : 'Unexpected repository error'

  return (
    <main className="min-h-screen bg-background px-4 py-8 text-foreground sm:px-6">
      <div className="mx-auto max-w-[760px] border-y border-border py-6">
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertTitle>Repository unavailable</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
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
