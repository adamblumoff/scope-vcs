import type { CommitSummary, RequestRevisions } from '@/api/types'
import type { RequestDiscussion } from './request-discussion-types'

export type RequestChangeSelection = {
  commit?: string
  revision?: string
}

export function requestChangeSelection(
  revisions: RequestRevisions['revisions'],
  search: RequestChangeSelection,
) {
  const latestVisibleRevision = [...revisions]
    .reverse()
    .find(({ commits }) => commits.length > 0)
  const revision = search.revision
    ? revisions.find(({ id }) => id === search.revision) ?? null
    : search.commit
      ? revisions.find((item) =>
          item.commits.some(({ oid }) => oid === search.commit)) ?? null
      : latestVisibleRevision ?? revisions.at(-1) ?? null
  const commit = revision && (
    search.commit && revision.commits.some(({ oid }) => oid === search.commit)
      ? search.commit
      : revision.commits.at(-1)?.oid
  ) || null
  return {
    commit,
    revision,
    unavailable: Boolean(
      (search.revision && !revision) ||
      (search.commit && commit !== search.commit),
    ),
  }
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
