import type { CommitSummary, RequestRevisions } from '@/api/types'
import type { RequestDiscussion } from './request-discussion-types'

export type RequestChangeSelection = {
  commit?: string
  revision?: string
}

export function requestChangeSelection(
  revisions: RequestRevisions['revisions'],
  reviewRevisionId: RequestRevisions['review_revision_id'],
  search: RequestChangeSelection,
) {
  const selectedRevisionId = search.revision ?? reviewRevisionId
  const revision = selectedRevisionId
    ? revisions.find(({ id }) => id === selectedRevisionId) ?? null
    : null
  const commit = revision && (
    search.commit && revision.commits.some(({ oid }) => oid === search.commit)
      ? search.commit
      : revision.commits.at(-1)?.oid
  ) || null
  const error = selectionError(revision, commit, search, selectedRevisionId)
  return {
    commit,
    error,
    revision,
  }
}

function selectionError(
  revision: RequestRevisions['revisions'][number] | null,
  commit: string | null,
  search: RequestChangeSelection,
  selectedRevisionId: string | null | undefined,
) {
  if (!revision) {
    return selectedRevisionId
      ? 'This revision or commit is not part of the request.'
      : null
  }
  if (search.commit && commit !== search.commit) {
    return 'This revision or commit is not part of the request.'
  }
  if (commit) return null
  if (revision.inspection === 'Unavailable') {
    return 'This revision could not be inspected within the request review limits.'
  }
  if (revision.inspection === 'Incomplete') {
    return 'This revision’s commit inspection is incomplete.'
  }
  return 'No commits in this revision are visible to you.'
}

export function orderedRequestCommits(
  revisions: RequestRevisions['revisions'],
): CommitSummary[] {
  return [...revisions].reverse().flatMap((revision) =>
    [...revision.commits].reverse().map((commit) => ({
      author: commit.author,
      change_count: commit.change_count,
      logical_commit_id: commit.oid,
      message: commit.message,
      parent_projected_id: commit.parent_oids[0] ?? null,
      projected_id: requestRevisionCommitId(revision.id, commit.oid),
    })))
}

export function requestRevisionPin(
  revision: RequestRevisions['revisions'][number] | null,
  commit: string | null,
  pinnedRevisionId: string | undefined,
): RequestChangeSelection | null {
  if (pinnedRevisionId || !revision) return null
  return {
    commit: commit ?? undefined,
    revision: revision.id,
  }
}

export function requestRevisionCommitId(revisionId: string, commitOid: string) {
  return `${revisionId}:${commitOid}`
}

export function requestCommitForListId(
  revisions: RequestRevisions['revisions'],
  listId: string,
) {
  for (const revision of revisions) {
    const commit = revision.commits.find(({ oid }) =>
      requestRevisionCommitId(revision.id, oid) === listId)
    if (commit) return { commitOid: commit.oid, revision }
  }
  return null
}

export function missingRequestCommitFileError(
  commit: RequestRevisions['revisions'][number]['commits'][number],
) {
  return commit.files_truncated
    ? 'This file is outside the bounded file list for the selected commit.'
    : 'This file is not part of the selected commit.'
}

export function discussionsForRequestCommit(
  discussions: RequestDiscussion[],
  revision: RequestRevisions['revisions'][number] | null,
  commitOid: string | null,
) {
  if (!revision || !commitOid) return []
  return discussions
    .filter((discussion) => {
      const anchor = discussion.anchor
      if (!anchor || anchor.revision_id !== revision.id) return false
      if (anchor.commit_oid) return anchor.commit_oid === commitOid
      return commitOid === revision.commits.at(-1)?.oid
    })
    .sort((left, right) => left.opened_position - right.opened_position)
}
