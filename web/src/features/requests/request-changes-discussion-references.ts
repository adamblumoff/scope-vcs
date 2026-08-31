import type { LoadDiscussionsInput } from './request-discussion-api'
import type { RequestDiscussionPage } from './request-discussion-types'

export type DiscussionReferenceQuery = {
  input: LoadDiscussionsInput
  key: string
}

export async function loadCompleteDiscussionReferencePage(
  input: LoadDiscussionsInput,
  loadPage: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>,
): Promise<RequestDiscussionPage> {
  const firstPage = await loadPage(input)
  const discussions = [...firstPage.discussions]
  const seenCursors = new Set<string>()
  let cursor = firstPage.next_cursor ?? undefined
  if (cursor) seenCursors.add(cursor)

  while (cursor) {
    const page = await loadPage({ ...input, cursor })
    if (page.snapshot_version !== firstPage.snapshot_version) {
      throw new Error('Discussion reference pagination changed snapshot version.')
    }
    discussions.push(...page.discussions)
    cursor = page.next_cursor ?? undefined
    if (cursor && seenCursors.has(cursor)) {
      throw new Error('Discussion reference pagination repeated a cursor.')
    }
    if (cursor) seenCursors.add(cursor)
  }

  return {
    discussions,
    next_cursor: null,
    snapshot_version: firstPage.snapshot_version,
  }
}

export async function loadCompleteDiscussionReferencePages(
  queries: DiscussionReferenceQuery[],
  loadPage: (input: LoadDiscussionsInput) => Promise<RequestDiscussionPage>,
  onError: (error: unknown) => void,
): Promise<Record<string, RequestDiscussionPage | null>> {
  const entries: Array<readonly [string, RequestDiscussionPage | null]> = []
  for (const query of queries) {
    const page = await loadCompleteDiscussionReferencePage(query.input, loadPage)
      .catch((error: unknown) => {
        onError(error)
        return null
      })
    entries.push([query.key, page])
  }
  return Object.fromEntries(entries)
}
