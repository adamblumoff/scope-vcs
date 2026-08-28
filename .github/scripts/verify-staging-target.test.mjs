import assert from 'node:assert/strict'
import { test } from 'node:test'
import { verifyStagingTarget, verifyStagingTopology } from './verify-staging-target.mjs'

function fixture() {
  const manifest = {
    railway: {
      databaseServiceId: 'database',
      environmentId: 'production',
      projectId: 'project',
      staging: {
        apiReplicas: 3,
        apiDomain: 'api-staging.example.test',
        cacheDomain: 'cache-staging.example.test',
        environmentId: 'staging',
        environmentName: 'staging',
        routerDomain: 'router-staging.example.test',
        routerReplicas: 1,
        routerServiceId: 'router',
        routerServiceName: 'scope-repo-router',
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
    { id: 'cache', name: 'scope-cache-service', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'worker', name: 'scope-worker', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'api', name: 'scope-api', status: 'SUCCESS', replicas: healthyReplicas(3) },
    { id: 'router', name: 'scope-repo-router', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'web', name: 'scope-web' },
  ]
  const status = {
    environments: { edges: [{ node: { id: 'staging', name: 'staging' } }] },
    id: 'project',
  }
  return { manifest, services, status }
}

function healthyReplicas(count) {
  return { configured: count, running: count, crashed: 0 }
}

test('accepts the reviewed staging target', () => {
  assert.deepEqual(verifyStagingTarget(fixture()), {
    apiReplicas: 3,
    productionEnvironmentId: 'production',
    projectId: 'project',
    routerReplicas: 1,
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
    (input) => { input.services = input.services.filter(({ id }) => id !== 'router') },
  ]) {
    const input = fixture()
    mutate(input)
    assert.throws(() => verifyStagingTarget(input))
  }
})

test('rejects invalid staging replica counts', () => {
  for (const [key, value] of [['apiReplicas', 0], ['routerReplicas', 1.5]]) {
    const input = fixture()
    input.manifest.railway.staging[key] = value
    assert.throws(() => verifyStagingTarget(input), /positive integer/)
  }
})

test('accepts only the reviewed healthy staging topology', () => {
  assert.doesNotThrow(() => verifyStagingTopology(fixture()))
  for (const mutate of [
    (input) => { input.services.find(({ id }) => id === 'api').replicas.running = 2 },
    (input) => { input.services.find(({ id }) => id === 'router').replicas.configured = 2 },
    (input) => { input.services.find(({ id }) => id === 'worker').status = 'CRASHED' },
    (input) => { input.services.find(({ id }) => id === 'cache').replicas.crashed = 1 },
  ]) {
    const input = fixture()
    mutate(input)
    assert.throws(() => verifyStagingTopology(input), /healthy replicas/)
  }
})

test('rejects missing or URL-shaped domains', () => {
  for (const domain of ['', 'https://api-staging.example.test/path']) {
    const input = fixture()
    input.manifest.railway.staging.apiDomain = domain
    assert.throws(() => verifyStagingTarget(input))
  }
})
