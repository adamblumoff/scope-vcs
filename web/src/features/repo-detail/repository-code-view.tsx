import type { RepoFile, RepoFileContent } from '@/api/types'
import { FileSystemTree } from '@/components/file-system-tree'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { FileCode2, FileQuestion, TriangleAlert } from 'lucide-react'

export function RepositoryCodeView({
  files,
  onSelectFile,
  selectedFile,
  selectedFileError,
  selectedPath,
}: {
  files: RepoFile[]
  onSelectFile: (file: RepoFile) => void
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedPath: string | null
}) {
  return (
    <section className="mt-8 border-y border-border">
      <div className="flex flex-wrap items-baseline justify-between gap-2 border-b border-border py-3">
        <div>
          <h2 className="text-sm font-semibold">Source</h2>
          <p className="mt-0.5 text-sm text-muted-foreground">
            Browse the latest scoped repository view.
          </p>
        </div>
        <Badge variant="neutral">
          {files.length} {files.length === 1 ? 'file' : 'files'}
        </Badge>
      </div>

      {files.length === 0 ? (
        <EmptySource
          description="Run scope push from the CLI to add files to this repository."
          title="No files yet"
        />
      ) : (
        <div className="grid min-w-0 lg:grid-cols-[minmax(280px,0.42fr)_minmax(0,1fr)]">
          <div className="min-w-0 border-b border-border py-2 lg:border-r lg:border-b-0 lg:pr-3">
            <FileSystemTree
              compactVisibility
              files={files}
              getFileMeta={fileStatus}
              metaColumnLabel="Status"
              onSelectFile={onSelectFile}
              selectedFilePath={selectedPath}
            />
          </div>
          <SourcePane
            error={selectedFileError}
            file={selectedFile}
            selectedPath={selectedPath}
          />
        </div>
      )}
    </section>
  )
}

function SourcePane({
  file,
  error,
  selectedPath,
}: {
  file: RepoFileContent | null
  error: string | null
  selectedPath: string | null
}) {
  if (!selectedPath) {
    return (
      <EmptySource
        description="Choose a file from the tree to inspect its projected contents."
        title="Select a file"
      />
    )
  }

  if (error) {
    return (
      <div className="flex min-h-52 items-center justify-center px-5 py-10 text-center" role="alert">
        <div className="max-w-sm">
          <TriangleAlert className="mx-auto size-5 text-destructive" />
          <h3 className="mt-3 text-sm font-semibold">Source unavailable</h3>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">{error}</p>
        </div>
      </div>
    )
  }

  if (!file) {
    return (
      <EmptySource
        description="This file is no longer available in the current scoped view."
        title="File unavailable"
      />
    )
  }

  return (
    <div className="min-w-0">
      <div className="flex min-w-0 flex-wrap items-center gap-2 border-b border-border px-4 py-3">
        <FileCode2 className="size-4 shrink-0 text-muted-foreground" />
        <h3 className="min-w-0 flex-1 break-all font-mono text-xs font-medium">
          {displayPath(file.path)}
        </h3>
        <VisibilityBadge visibility={file.visibility} />
      </div>
      {file.content.kind === 'text' ? (
        isReadme(file.path) ? (
          <pre className="max-h-[70vh] overflow-auto p-5 font-sans text-sm leading-6 whitespace-pre-wrap text-pretty">
            {file.content.text}
          </pre>
        ) : (
          <pre className="max-h-[70vh] overflow-auto p-4 font-mono text-xs leading-5 whitespace-pre">
            <code>{file.content.text}</code>
          </pre>
        )
      ) : (
        <EmptySource
          description={`${formatBytes(file.content.size_bytes)} · ${file.content.oid.slice(0, 12)}`}
          title="Binary file not rendered"
        />
      )}
    </div>
  )
}

function EmptySource({ description, title }: { description: string; title: string }) {
  return (
    <div className="flex min-h-52 items-center justify-center px-5 py-10 text-center">
      <div className="max-w-sm">
        <FileQuestion className="mx-auto size-5 text-muted-foreground" />
        <h3 className="mt-3 text-sm font-semibold">{title}</h3>
        <p className="mt-1 text-sm leading-5 text-muted-foreground">{description}</p>
      </div>
    </div>
  )
}

function fileStatus(file: RepoFile) {
  return <span className="text-muted-foreground">{file.tracked ? 'Tracked' : 'Missing'}</span>
}

function displayPath(path: string) {
  return path.replace(/^\/+/, '') || '/'
}

function isReadme(path: string) {
  return /(^|\/)readme(?:\.[^/]+)?$/i.test(path)
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
