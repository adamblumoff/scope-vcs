import * as assert from 'node:assert/strict'
import { test } from 'node:test'
import { arrayOf } from './http'
import { apiValidators } from './validators.generated'

test('generated validators follow enum, optional, and unknown-field Serde policy', () => {
  assert.equal(apiValidators.ErrorResponse({
    code: 'internal',
    message: 'failed',
    retryable: false,
  }), true)
  assert.equal(apiValidators.ErrorResponse({
    code: 'internal',
    extra_server_field: 'allowed until Rust denies unknown fields',
    message: 'failed',
    retryable: false,
  }), true)
  assert.equal(apiValidators.ErrorResponse({
    code: 'not-a-real-code',
    message: 'failed',
    retryable: false,
  }), false)
  assert.equal(apiValidators.ErrorResponse({
    code: 'internal',
    message: 'failed',
  }), false)
})

test('generated validators enforce arrays and JavaScript safe integers', () => {
  const validateRepoFiles = arrayOf(apiValidators.RepoFileResponse)
  assert.equal(validateRepoFiles([{
    oid: '0123456789abcdef0123456789abcdef01234567',
    path: '/README.md',
    tracked: true,
    visibility: 'Public',
  }]), true)
  assert.equal(validateRepoFiles([{
    oid: '0123456789abcdef0123456789abcdef01234567',
    path: '/README.md',
    tracked: 'yes',
    visibility: 'Public',
  }]), false)

  const connected = {
    incarnation_id: 'incarnation-1',
    kind: 'Connected',
    repo_id: 'owner/repo',
    version: Number.MAX_SAFE_INTEGER,
  }
  assert.equal(apiValidators.RepoChangeEvent(connected), true)
  assert.equal(apiValidators.RepoChangeEvent({
    ...connected,
    version: Number.MAX_SAFE_INTEGER + 1,
  }), false)
})

test('run mutation responses use the exact RunResponse contract', () => {
  const response = {
    cancellation_requested: true,
    completed_at_unix: 2,
    created_at_unix: 1,
    git_oid: 'a'.repeat(40),
    id: 'run-1',
    logs_truncated: false,
    repository_id: 'owner/repo',
    state: 'canceled',
    updated_at_unix: 2,
    workflow_name: 'checks',
  }

  assert.equal(apiValidators.RunResponse(response), true)
  assert.equal(apiValidators.RunResponse({
    ...response,
    state: 'not-a-run-state',
  }), false)
})

test('request commit responses require explicit file truncation state', () => {
  const commit = {
    author: null,
    authored_at_unix: 1,
    change_count: 10_001,
    files: [],
    files_truncated: true,
    message: 'Large commit',
    oid: 'a'.repeat(40),
    parent_oids: ['b'.repeat(40)],
  }
  const response = {
    has_earlier_revisions: false,
    review_revision_id: 'revision-1',
    revisions: [{
      actor: { handle: 'scope', id: 'user-1' },
      commits: [commit],
      created_at_unix: 1,
      id: 'revision-1',
      inspection: 'Incomplete',
      new_head_oid: 'a'.repeat(40),
      old_head_oid: null,
      position: 1,
    }],
  }

  assert.equal(apiValidators.RequestRevisionListResponse(response), true)
  const { files_truncated: _, ...missingState } = commit
  response.revisions[0].commits = [missingState as unknown as typeof commit]
  assert.equal(apiValidators.RequestRevisionListResponse(response), false)
})
