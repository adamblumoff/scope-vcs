import type { RequestChangeBlockResponse } from '@/api/types.generated'
import type { RequestChangeBlockFiles } from '@/api/types'
import type { LoadRequestChangeBlockFilesInput } from '@/api/requests'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  GitCompareArrows,
  LoaderCircle,
} from 'lucide-react'
import { useState } from 'react'

export function RequestChangeBlock({
  block,
  loadFiles: loadChangeBlockFiles,
  params,
}: {
  block: RequestChangeBlockResponse
  loadFiles: (
    input: LoadRequestChangeBlockFilesInput,
  ) => Promise<RequestChangeBlockFiles>
  params: { owner: string; repo: string; request_id: string }
}) {
  const [expanded, setExpanded] = useState(false)
  const [files, setFiles] = useState<RequestChangeBlockFiles | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  async function toggle() {
    if (expanded) {
      setExpanded(false)
      return
    }
    setExpanded(true)
    if (files || loading) return
    await fetchFiles()
  }

  async function fetchFiles() {
    setLoading(true)
    setError(null)
    try {
      setFiles(await loadChangeBlockFiles({
        ...params,
        block_id: block.id,
      }))
    } catch (loadError) {
      setError(
        loadError instanceof Error
          ? loadError.message
          : 'Files could not be loaded.',
      )
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="mt-3">
      <button
        aria-expanded={expanded}
        className="-mx-2 flex w-[calc(100%+1rem)] items-center gap-2 rounded-md px-2 py-2 text-left hover:bg-brand-muted"
        onClick={() => void toggle()}
        type="button"
      >
        <span className="sr-only">Files in update</span>
        <GitCompareArrows className="size-4 shrink-0 text-brand" />
        <span className="font-mono text-xs text-muted-foreground">
          {shortOid(block.old_head_oid)} → {shortOid(block.new_head_oid)}
        </span>
        {files ? (
          <span className="ml-auto text-xs font-medium text-muted-foreground">
            {files.files.length} {files.files.length === 1 ? 'file' : 'files'}
          </span>
        ) : (
          <span className="ml-auto" />
        )}
        {expanded ? (
          <ChevronDown className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-4 shrink-0 text-muted-foreground" />
        )}
      </button>

      {expanded ? (
        <div className="mt-1 border-t border-brand/25 pt-3">
          {loading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin" />
              Loading files…
            </div>
          ) : null}
          {error ? (
            <div className="flex items-center justify-between gap-3 text-sm text-destructive">
              <span>{error}</span>
              <Button
                disabled={loading}
                onClick={() => void fetchFiles()}
                size="sm"
                variant="secondary"
              >
                Retry
              </Button>
            </div>
          ) : null}
          {files ? (
            <div className="divide-y divide-border">
              {files.files.map((file) => (
                <div
                  className="flex min-w-0 items-center gap-3 py-2 first:pt-0 last:pb-0"
                  key={file.path}
                >
                  <span
                    aria-label={file.kind}
                    className={cn(
                      'w-4 shrink-0 text-center font-mono text-sm font-semibold',
                      statusTone(file.kind),
                    )}
                    title={file.kind}
                  >
                    {statusSymbol(file.kind)}
                  </span>
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">
                    {file.path}
                  </span>
                  <a
                    aria-label={`Open diff for ${file.path} in a new tab`}
                    className="inline-flex size-7 shrink-0 items-center justify-center rounded-md text-brand hover:bg-background/70 hover:text-foreground"
                    href={historyHref(params, block.id, file.path)}
                    rel="noopener noreferrer"
                    target="_blank"
                    title="Open diff in a new tab"
                  >
                    <ExternalLink className="size-3" />
                  </a>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function historyHref(
  params: { owner: string; repo: string; request_id: string },
  revision: string,
  path: string,
) {
  const query = new URLSearchParams({
    request: params.request_id,
    revision,
    path,
  })
  return `/repos/${encodeURIComponent(params.owner)}/${encodeURIComponent(params.repo)}/history?${query}`
}

function shortOid(oid: string) {
  return oid.slice(0, 8)
}

function statusTone(kind: string) {
  if (kind === 'Added') return 'text-green-700 dark:text-green-400'
  if (kind === 'Deleted') return 'text-destructive'
  return 'text-amber-700 dark:text-amber-400'
}

function statusSymbol(kind: string) {
  if (kind === 'Added') return '+'
  if (kind === 'Deleted') return '−'
  return '±'
}
