import assert from 'node:assert/strict'
import test from 'node:test'
import {
  countLines,
  evaluateSourceSizes,
  isSourceFile,
  sourceCategory,
} from './check-source-size.mjs'

test('counts physical lines without inventing one after a trailing newline', () => {
  assert.equal(countLines(''), 0)
  assert.equal(countLines('one'), 1)
  assert.equal(countLines('one\ntwo\n'), 2)
})

test('classifies production separately from tests and support', () => {
  assert.equal(sourceCategory('api/src/http/runs.rs'), 'production')
  assert.equal(sourceCategory('api/src/workflow_tests/runs.rs'), 'test/support')
  assert.equal(sourceCategory('crates/scope-postgres/src/db/requests/tests.rs'), 'test/support')
  assert.equal(sourceCategory('crates/example/src/model_tests.rs'), 'test/support')
  assert.equal(sourceCategory('web/smoke/repository.spec.mjs'), 'test/support')
  assert.equal(sourceCategory('dev/check'), 'test/support')
})

test('ignores generated and non-source files', () => {
  assert.equal(isSourceFile('web/src/routeTree.gen.ts'), false)
  assert.equal(isSourceFile('web/src/api/types.generated.ts'), false)
  assert.equal(isSourceFile('crates/scope-postgres/src/db/generated_ids.rs'), true)
  assert.equal(isSourceFile('dev/scope-dev'), true)
  assert.equal(isSourceFile('Cargo.lock'), false)
  assert.equal(isSourceFile('docs/architecture.md'), false)
  assert.equal(isSourceFile('api/src/main.rs'), true)
})

test('requires reviewed production ownership and preserves the hard cap', () => {
  const sources = [
    { category: 'production', lines: 699, path: 'src/small.rs' },
    { category: 'production', lines: 700, path: 'src/reviewed.rs' },
    { category: 'production', lines: 1000, path: 'src/capped.rs' },
    { category: 'test/support', lines: 700, path: 'tests/large.rs' },
    { category: 'test/support', lines: 1001, path: 'tests/too-large.rs' },
  ]
  const audit = [
    { owner: 'reviewed behavior', path: 'src/reviewed.rs', reason: 'cohesive parser' },
    { owner: 'capped behavior', path: 'src/capped.rs', reason: 'single transition table' },
  ]
  const result = evaluateSourceSizes(sources, audit)
  assert.deepEqual(result.production.map(({ path }) => path), ['src/reviewed.rs', 'src/capped.rs'])
  assert.deepEqual(result.support.map(({ path }) => path), ['tests/large.rs', 'tests/too-large.rs'])
  assert.deepEqual(result.errors, ['tests/too-large.rs: 1001 lines exceeds the 1000-line cap'])
})

test('rejects missing, stale, duplicate, and malformed audit entries', () => {
  const sources = [
    { category: 'production', lines: 700, path: 'src/unowned.rs' },
    { category: 'production', lines: 699, path: 'src/stale.rs' },
    { category: 'test/support', lines: 800, path: 'tests/not-production.rs' },
  ]
  const result = evaluateSourceSizes(sources, [
    { owner: 'old', path: 'src/stale.rs', reason: 'old reason' },
    { owner: 'tests', path: 'tests/not-production.rs', reason: 'wrong category' },
    { owner: 'missing', path: 'src/missing.rs', reason: 'gone' },
    { owner: 'duplicate', path: 'src/missing.rs', reason: 'still gone' },
    { owner: '', path: 'src/bad.rs', reason: '' },
  ])
  assert.ok(result.errors.some((error) => error.includes('src/unowned.rs')))
  assert.ok(result.errors.some((error) => error.includes('src/stale.rs')))
  assert.ok(result.errors.some((error) => error.includes('tests/not-production.rs')))
  assert.ok(result.errors.some((error) => error.includes('duplicate source-size audit')))
  assert.ok(result.errors.some((error) => error.includes('non-empty path, owner, and reason')))
})
