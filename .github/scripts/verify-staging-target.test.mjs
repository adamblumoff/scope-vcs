import assert from 'node:assert/strict'
import { test } from 'node:test'
import { verifyStagingTarget } from './verify-staging-target.mjs'

function fixture() {
  const manifest = {
    railway: {
      databaseServiceId: 'database',
      environmentId: 'production',
      projectId: 'project',
      staging: {
        apiDomain: 'api-staging.example.test',
        cacheDomain: 'cache-staging.example.test',
        environmentId: 'staging',
        environmentName: 'staging',
        webDomain: 'web-staging.example.test',
      },
    },
    services: {
      api: { id: 'api', name: 'scope-api' },
      cache: { id: 'cache', name: 'scope-cache-service' },
      web: { id: 'web', name: 'scope-web' },
      worker: { id: 'worker', name: 'scope-worker' },
    },
  }
  const services = [
    { id: 'database', name: 'scope-postgres' },
    { id: 'cache', name: 'scope-cache-service' },
    { id: 'worker', name: 'scope-worker' },
    { id: 'api', name: 'scope-api' },
    { id: 'web', name: 'scope-web' },
  ]
  const status = {
    environments: { edges: [{ node: { id: 'staging', name: 'staging' } }] },
    id: 'project',
  }
  return { manifest, services, status }
}

test('accepts the reviewed staging target', () => {
  assert.deepEqual(verifyStagingTarget(fixture()), {
    productionEnvironmentId: 'production',
    projectId: 'project',
    stagingEnvironmentId: 'staging',
    stagingEnvironmentName: 'staging',
  })
})

test('rejects production as staging', () => {
  const input = fixture()
  input.manifest.railway.staging.environmentId = 'production'
  assert.throws(() => verifyStagingTarget(input), /differ from production/)
})

test('rejects a different project, environment, or service', () => {
  for (const mutate of [
    (input) => { input.status.id = 'wrong' },
    (input) => { input.status.environments.edges[0].node.id = 'wrong' },
    (input) => { input.services = input.services.filter(({ id }) => id !== 'api') },
  ]) {
    const input = fixture()
    mutate(input)
    assert.throws(() => verifyStagingTarget(input))
  }
})

test('rejects missing or URL-shaped domains', () => {
  for (const domain of ['', 'https://api-staging.example.test/path']) {
    const input = fixture()
    input.manifest.railway.staging.apiDomain = domain
    assert.throws(() => verifyStagingTarget(input))
  }
})
