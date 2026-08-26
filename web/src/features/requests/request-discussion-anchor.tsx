import { Link } from '@tanstack/react-router'
import { GitCommit } from 'lucide-react'
import type { RequestDiscussion } from './request-discussion-types'
import { shortOid } from './request-labels'

/**
 * Where a discussion was opened. The revision ordinal is the part readers
 * reason about, so it leads and never truncates; the path gives up its
 * leading directories first so the filename survives.
 */
export function RequestDiscussionAnchor({
  anchor,
  params,
}: {
  anchor: NonNullable<RequestDiscussion['anchor']>
  params: { owner: string; repo: string; request_id: string }
}) {
  return (
    <Link
      className="mt-2 inline-flex max-w-full items-center gap-2 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs text-muted-foreground hover:border-input hover:text-foreground"
      params={{
        owner: params.owner,
        repo: params.repo,
        requestId: params.request_id,
      }}
      search={{
        commit: anchor.commit_oid ?? undefined,
        path: anchor.path ?? undefined,
        revision: anchor.revision_id,
      }}
      to="/$owner/$repo/requests/$requestId/changes"
    >
      <GitCommit className="size-3.5 shrink-0" />
      <span className="shrink-0 font-semibold text-foreground">
        Revision {anchor.revision_position}
      </span>
      {anchor.commit_oid ? (
        <span className="shrink-0">{shortOid(anchor.commit_oid)}</span>
      ) : null}
      {anchor.path ? (
        <span className="truncate" dir="rtl">
          {anchor.path.replace(/^\/+/, '')}
        </span>
      ) : null}
    </Link>
  )
}
