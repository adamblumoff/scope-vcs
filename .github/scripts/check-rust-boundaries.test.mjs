import assert from 'node:assert/strict'
import test from 'node:test'
import {
  validateRustBoundaries,
  validateSourceLayout,
  validateStandaloneManifests,
} from './check-rust-boundaries.mjs'

const cliRoot = '/repo/cli'

function dependency(name, kind = null) {
  return { kind, name, source: null }
}

function packageFixture(name, dependencies = []) {
  return { dependencies, id: `path+file:///repo/${name}#0.1.0`, name }
}

function metadata(packages, workspaceRoot = '/repo') {
  return {
    packages,
    workspace_members: packages.map(({ id }) => id),
    workspace_root: workspaceRoot,
  }
}

function validMainMetadata() {
  return metadata([
    packageFixture('scope-domain'),
    packageFixture('scope-cache-domain'),
    packageFixture('scope-git-process'),
    packageFixture('scope-api-contract', [dependency('scope-domain')]),
    packageFixture('scope-cache-contract', [dependency('scope-cache-domain')]),
    packageFixture('scope-content-lifecycle', [dependency('scope-domain')]),
    packageFixture('api', [dependency('scope-content-lifecycle')]),
    packageFixture('worker', [dependency('scope-content-lifecycle')]),
    packageFixture('scope-cache-service'),
    packageFixture('scope-repo-router'),
    packageFixture('scope-runner-runtime'),
  ])
}

function validCliMetadata() {
  return metadata([
    packageFixture('scope-cli', [dependency('scope-domain'), dependency('scope-api-contract')]),
  ], cliRoot)
}

test('accepts domain leaves, narrow contracts, shared orchestration, and standalone CLI', () => {
  const result = validateRustBoundaries(validMainMetadata(), validCliMetadata(), cliRoot)
  assert.deepEqual(result.errors, [])
  assert.deepEqual(result.lifecycleConsumers, ['api', 'worker'])
})

test('rejects outward domain dependencies and reusable-to-application edges', () => {
  const main = validMainMetadata()
  main.packages.find(({ name }) => name === 'scope-domain').dependencies.push(dependency('scope-api-contract'))
  main.packages.find(({ name }) => name === 'scope-content-lifecycle').dependencies.push(dependency('api'))
  main.packages.find(({ name }) => name === 'scope-content-lifecycle').dependencies.push(dependency('scope-repo-router'))
  const result = validateRustBoundaries(main, validCliMetadata(), cliRoot)
  assert.ok(result.errors.some((error) => error.includes('scope-domain: domain leaf')))
  assert.ok(result.errors.some((error) => error.includes('reusable package depends on application api')))
  assert.ok(result.errors.some((error) => error.includes('scope-repo-router')))
})

test('rejects broad contracts and orchestration without two real consumers', () => {
  const main = validMainMetadata()
  main.packages.find(({ name }) => name === 'scope-api-contract').dependencies.push(dependency('scope-content-lifecycle'))
  main.packages.find(({ name }) => name === 'worker').dependencies = []
  const result = validateRustBoundaries(main, validCliMetadata(), cliRoot)
  assert.ok(result.errors.some((error) => error.includes('scope-api-contract: contract depends on')))
  assert.ok(result.errors.some((error) => error.includes('expected API and worker consumers')))
})

test('ignores dev-only test support and protects CLI independence', () => {
  const main = validMainMetadata()
  main.packages.find(({ name }) => name === 'scope-cache-service').dependencies.push(dependency('scope-domain', 'dev'))
  const cli = validCliMetadata()
  cli.workspace_root = '/repo'
  cli.packages[0].dependencies.push(dependency('scope-content-lifecycle'))
  const result = validateRustBoundaries(main, cli, cliRoot)
  assert.ok(!result.errors.some((error) => error.includes('scope-cache-service')))
  assert.ok(result.errors.some((error) => error.includes('workspace root')))
  assert.ok(result.errors.some((error) => error.includes('unexpected local dependency')))
})

test('keeps CLI-staged dependency manifests self-contained', () => {
  assert.deepEqual(validateStandaloneManifests({
    'scope-api-contract': '[package]\nversion = "0.1.0"\n',
    'scope-domain': '[dependencies]\nserde = "1"\n',
  }), [])
  assert.deepEqual(validateStandaloneManifests({
    'scope-domain': '[package]\nversion.workspace = true\n',
    'scope-api-contract': '[dependencies]\nscope_domain.workspace = true\n',
  }), [
    'scope-domain: manifest must stay self-contained for standalone CLI staging',
    'scope-api-contract: manifest must stay self-contained for standalone CLI staging',
  ])
})

test('requires behavior-owned source homes and rejects retired catch-alls', () => {
  const current = new Set([
    'api/src/use_cases/content_cleanup.rs',
    'api/src/use_cases/git_receive/mod.rs',
    'api/src/use_cases/request_discussion_mutation.rs',
    'api/src/use_cases/request_merge.rs',
    'api/src/use_cases/run_inspection.rs',
    'crates/scope-domain/src/repository/mod.rs',
    'crates/scope-domain/src/reviewed_updates/mod.rs',
    'crates/scope-domain/src/runs/cache/mod.rs',
    'crates/scope-domain/src/runs/workflow/mod.rs',
    'crates/scope-postgres/src/db/cleanup_queue/mod.rs',
    'runner-runtime/src/api/mod.rs',
    'runner-runtime/src/cache/mod.rs',
    'runner-runtime/src/workflow.rs',
  ])
  assert.deepEqual(validateSourceLayout(current), [])

  current.delete('api/src/use_cases/request_merge.rs')
  current.add('api/src/git/request_merge.rs')
  const errors = validateSourceLayout(current)
  assert.ok(errors.some((error) => error.includes('required behavior-owned source home is missing')))
  assert.ok(errors.some((error) => error.includes('retired catch-all source home was reintroduced')))
})
