import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'
import * as parsers from './request-inputs'

const request = { owner: 'scope', repo: 'vcs', request_id: 'req_1' }
const discussion = { ...request, discussion_id: 'discussion_1' }

test('request inputs reject non-objects and missing required identifiers', () => {
  for (const input of [null, undefined, [], 'request', 1, {}, { ...request, request_id: ' ' }, { ...request, request_id: 1 }]) {
    assert.throws(() => parsers.parseRequestParams(input))
  }
  assert.deepEqual(parsers.parseRequestParams({ ...request, extra: 'discard' }), request)
  assert.throws(() => parsers.parseDiscussionActionInput(request))
  assert.throws(() => parsers.parseLoadRequestRevisionDiffInput({ ...request, path: '/a' }))
})

test('files preserve significant whitespace and reject invalid path values', () => {
  assert.equal(parsers.parseRepoFileInput({ ...request, path: '/ file ' }).path, '/ file ')
  for (const path of ['', ' ', null, 10, '/file\0name', 'x'.repeat(4097)]) {
    assert.throws(() => parsers.parseRepoFileInput({ ...request, path }))
  }
  assert.equal(parsers.parseLoadRequestRevisionDiffInput({ ...request, revision_id: 'rev', commit_oid: 'oid', path: '/a' }).path, '/a')
})

test('pagination and booleans reject coercion, overflow, and invalid bounds', () => {
  for (const limit of [0, -1, 101, 1.5, NaN, Infinity, '25', null]) {
    assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, limit }))
  }
  for (const after of [-1, 0.1, Infinity, Number.MAX_SAFE_INTEGER + 1, '0', undefined]) {
    assert.throws(() => parsers.parseLoadDiscussionChangesInput({ ...request, after }))
  }
  assert.equal(parsers.parseLoadDiscussionChangesInput({ ...request, after: 0 }).after, 0)
  assert.equal(parsers.parseLoadDiscussionsInput({ ...request, limit: 100, include_revision_anchor: false }).limit, 100)
  assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, include_revision_anchor: 'false' }))
  assert.throws(() => parsers.parseLoadRepliesInput({ ...discussion, before: -1 }))
  assert.throws(() => parsers.parseMarkDiscussionReadInput({ ...discussion, through_position: '1' }))
  assert.throws(() => parsers.parseLoadDiscussionsInput({ ...request, cursor: {} }))
})

test('request actions validate the action and only require handles for invitee actions', () => {
  for (const action of ['close', 'leave', 'merge', 'submit']) {
    assert.deepEqual(parsers.parseRequestActionInput({ ...request, action }), { ...request, action })
  }
  for (const action of ['add_invitee', 'remove_invitee']) {
    assert.throws(() => parsers.parseRequestActionInput({ ...request, action }))
    assert.equal(parsers.parseRequestActionInput({ ...request, action, handle: 'adam' }).action, action)
  }
  assert.throws(() => parsers.parseRequestActionInput({ ...request, action: 'delete' }))
})

test('ratings enforce integer scores and UTF-8 reason limits', () => {
  for (const score of [0, 6, 1.5, '5', NaN]) assert.throws(() => parsers.parseRateRequestInput({ ...request, score, reason: 'Good' }))
  assert.equal(parsers.parseRateRequestInput({ ...request, score: 5, reason: 'é'.repeat(512) }).score, 5)
  for (const reason of ['', ' ', 'é'.repeat(513)]) assert.throws(() => parsers.parseRateRequestInput({ ...request, score: 5, reason }))
})

test('discussion and reply payloads validate identifiers, anchors, and body byte limits', () => {
  const create = { ...request, anchor: null, client_discussion_id: 'client', body_markdown: '  hello\n' }
  assert.deepEqual(parsers.parseCreateDiscussionInput(create), create)
  assert.equal(parsers.parseCreateDiscussionInput({ ...create, body_markdown: 'é'.repeat(32768) }).body_markdown.length, 32768)
  for (const patch of [{ body_markdown: 'é'.repeat(32769) }, { body_markdown: ' ' }, { anchor: {} }, { client_discussion_id: 'x'.repeat(129) }]) {
    assert.throws(() => parsers.parseCreateDiscussionInput({ ...create, ...patch }))
  }
  const anchored = { ...create, anchor: { revision_id: 'rev', commit_oid: null, path: null } }
  assert.deepEqual(parsers.parseCreateDiscussionInput(anchored), anchored)
  const reply = { ...discussion, body_markdown: 'reply', client_reply_id: 'client', reply_to_reply_id: null }
  assert.deepEqual(parsers.parseCreateReplyInput(reply), reply)
  assert.throws(() => parsers.parseCreateReplyInput({ ...reply, reply_to_reply_id: 1 }))
  assert.equal(parsers.parseUpdateDescriptionInput({ ...request, description_markdown: '' }).description_markdown, '')
  assert.throws(() => parsers.parseUpdateDescriptionInput({ ...request, description_markdown: 'x'.repeat(256 * 1024 + 1) }))
})

test('request and file server functions use named unknown-input parsers', () => {
  const routes = [
    '$owner.$repo.requests.$requestId.tsx',
    '$owner.$repo.requests.$requestId.index.tsx',
    '$owner.$repo.requests.$requestId.changes.tsx',
    '$owner.$repo._code.index.tsx',
  ]
  for (const route of routes) {
    const source = readFileSync(resolve('src/routes', route), 'utf8')
    const validators = [...source.matchAll(/\.validator\(([^\n]*)\)/g)].map((match) => match[1])
    assert.ok(validators.length > 0, route)
    for (const validator of validators) assert.match(validator, /^parse[A-Za-z]+$/, `${route}: ${validator}`)
    assert.equal((source.match(/createServerFn\(/g) ?? []).length, validators.length, route)
  }
})
