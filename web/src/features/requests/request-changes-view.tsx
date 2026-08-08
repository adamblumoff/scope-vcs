import type {
  RequestRevisionCommitFiles,
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type { LoadRequestRevisionCommitInput } from '@/api/requests'
import { GitCommit } from 'lucide-react'
import {
  RequestChangesWorkbench,
  type RequestChangesSearch,
} from './request-changes-workbench'
import type {
  LoadDiscussionsInput,
} from './request-discussion-api'
import type { RequestDiscussionPage } from './request-discussion-types'

type RequestChangesViewProps = {
  audience: 'private' | 'public'
  loadCommit: (
    input: LoadRequestRevisionCommitInput,
  ) => Promise<RequestRevisionCommitFiles>
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>
  onSearchChange: (search: RequestChangesSearch) => void
  params: {
    owner: string
    repo: string
    request_id: string
  }
  repoId: string
  revisions: RequestRevisions | null
  search: RequestChangesSearch
}

export function RequestChangesView({
  audience,
  loadCommit,
  loadDiff,
  loadDiscussions,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: RequestChangesViewProps) {
  if (!revisions) {
    return (
      <section className="border-b border-border px-5 py-14 text-center lg:px-7">
        <GitCommit className="mx-auto size-5 text-muted-foreground" />
        <h2 className="mt-3 text-sm font-semibold">Changes are unavailable</h2>
        <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
          The request conversation is still available. Refresh to try loading its revision history again.
        </p>
      </section>
    )
  }

  return (
    <RequestChangesWorkbench
      audience={audience}
      loadCommit={loadCommit}
      loadDiff={loadDiff}
      loadDiscussions={loadDiscussions}
      onSearchChange={onSearchChange}
      params={params}
      repoId={repoId}
      revisions={revisions}
      search={search}
    />
  )
}
