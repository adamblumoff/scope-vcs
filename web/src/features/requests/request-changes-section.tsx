import type {
  CommitFile,
  RequestChanges,
  ReviewFileDiff,
} from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { Badge } from '@/components/ui/badge'
import { ReviewFileDiffDrawer } from '@/features/review/review-file-diff-drawer'
import { FileDiff, Files } from 'lucide-react'

export function RequestChangesSection({
  changes,
  error,
  onSelectFile,
  selectedDiff,
  selectedDiffError,
  selectedPath,
}: {
  changes: RequestChanges | null
  error: string | null
  onSelectFile: (path: string) => void
  selectedDiff: ReviewFileDiff | null
  selectedDiffError: string | null
  selectedPath: string | null
}) {
  if (error) {
    return (
      <section className="mt-6 border-y border-border py-10 text-center" role="alert">
        <FileDiff className="mx-auto size-5 text-destructive" />
        <h2 className="mt-3 text-sm font-semibold">Changes unavailable</h2>
        <p className="mx-auto mt-1 max-w-md text-sm leading-5 text-muted-foreground">
          {error}
        </p>
      </section>
    )
  }

  if (!changes || changes.files.length === 0) {
    return (
      <section className="mt-6 border-y border-border py-10 text-center">
        <Files className="mx-auto size-5 text-muted-foreground" />
        <h2 className="mt-3 text-sm font-semibold">No uploaded changes</h2>
        <p className="mx-auto mt-1 max-w-md text-sm leading-5 text-muted-foreground">
          Push a request revision to compare it with the request base.
        </p>
      </section>
    )
  }

  return (
    <section className="mt-6 border-y border-border">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border py-3">
        <div>
          <h2 className="text-sm font-semibold">Changed files</h2>
          <p className="mt-0.5 text-sm text-muted-foreground">
            Compared with the request base commit.
          </p>
        </div>
        <Badge variant="neutral">{changes.files.length}</Badge>
      </div>
      <div className="grid min-w-0 xl:grid-cols-[minmax(300px,0.38fr)_minmax(0,1fr)]">
        <div className="min-w-0 border-b border-border py-2 xl:border-r xl:border-b-0 xl:pr-3">
          <FileSystemTree
            files={changes.files}
            getFileMeta={changeKind}
            metaColumnLabel="Change"
            onSelectFile={(file) => onSelectFile(file.path)}
            selectedFilePath={selectedPath}
          />
        </div>
        {selectedPath ? (
          <ReviewFileDiffDrawer
            diff={selectedDiff}
            error={selectedDiffError}
            loading={false}
            selectedPath={selectedPath}
          />
        ) : (
          <div className="flex min-h-72 items-center justify-center px-5 py-10 text-center">
            <div className="max-w-sm">
              <FileDiff className="mx-auto size-5 text-muted-foreground" />
              <h3 className="mt-3 text-sm font-semibold">Select a changed file</h3>
              <p className="mt-1 text-sm leading-5 text-muted-foreground">
                Choose a file to inspect its unified diff.
              </p>
            </div>
          </div>
        )}
      </div>
    </section>
  )
}

function changeKind(file: CommitFile) {
  const variant =
    file.kind === 'Added'
      ? 'success'
      : file.kind === 'Deleted'
        ? 'danger'
        : 'neutral'
  return <Badge variant={variant}>{file.kind}</Badge>
}
