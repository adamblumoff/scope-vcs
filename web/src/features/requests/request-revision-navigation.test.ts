import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestRevisions } from '@/api/types'
import { createRequestRevisionRedirectHandoff } from './request-revision-navigation'

const input = {
  commit_oid: 'commit-2',
  owner: 'adam',
  repo: 'scope',
  request_id: 'request-1',
  revision_id: 'revision-2',
}
const revisions = {
  has_earlier_revisions: false,
  review_revision_id: 'revision-2',
  revisions: [],
} satisfies RequestRevisions

test('hands the initial revision response to its canonical redirect once', () => {
  const handoff = createRequestRevisionRedirectHandoff()

  handoff.stage(input, revisions)

  assert.equal(handoff.take(input), revisions)
  assert.equal(handoff.take(input), null)
})

test('does not reuse a revision response for another selection', () => {
  const handoff = createRequestRevisionRedirectHandoff()

  handoff.stage(input, revisions)

  assert.equal(
    handoff.take({ ...input, commit_oid: 'another-commit' }),
    null,
  )
  assert.equal(handoff.take(input), null)
})
