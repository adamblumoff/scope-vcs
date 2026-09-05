import type {
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type { LoadRequestRevisionCommitInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { GitCommit } from 'lucide-react'
import {
  RequestChangesWorkbench,
  type RequestChangesDiscussionReferences,
  type RequestChangesSearch,
} from './request-changes-workbench'
import type {
  LoadDiscussionsInput,
} from './request-discussion-api'
import type { RequestDiscussionPage } from './request-discussion-types'

type RequestChangesViewProps = {
  audience: 'private' | 'public'
  initialDiscussionReferences: RequestChangesDiscussionReferences
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
    signal?: AbortSignal,
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
  initialDiscussionReferences,
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
      <EmptyState
        description="The request conversation is still available. Reload the page to try loading its revision history again."
        icon={<GitCommit />}
        title="Changes are unavailable"
      />
    )
  }

  return (
    <RequestChangesWorkbench
      audience={audience}
      initialDiscussionReferences={initialDiscussionReferences}
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
