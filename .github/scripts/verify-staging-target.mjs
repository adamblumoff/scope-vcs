import assert from 'node:assert/strict'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const requiredServices = ['cache', 'worker', 'api', 'web']

export function verifyStagingTarget({ manifest, services, status }) {
  const railway = manifest?.railway
  assertObject(railway, 'manifest.railway')
  assertObject(railway.staging, 'manifest.railway.staging')

  const projectId = requiredString(railway.projectId, 'Railway project ID')
  const productionEnvironmentId = requiredString(
    railway.environmentId,
    'Railway production environment ID',
  )
  const stagingEnvironmentId = requiredString(
    railway.staging.environmentId,
    'Railway staging environment ID',
  )
  const stagingEnvironmentName = requiredString(
    railway.staging.environmentName,
    'Railway staging environment name',
  )
  const apiReplicas = requiredPositiveInteger(
    railway.staging.apiReplicas,
    'staging API replicas',
  )
  const routerReplicas = requiredPositiveInteger(
    railway.staging.routerReplicas,
    'staging router replicas',
  )
  assert.notEqual(
    stagingEnvironmentId,
    productionEnvironmentId,
    'Railway staging environment must differ from production',
  )
  assert.equal(status?.id, projectId, 'Railway project does not match the manifest')

  const environments = status?.environments?.edges?.map(({ node }) => node) ?? []
  assert(
    environments.some(
      ({ id, name }) =>
        id === stagingEnvironmentId && name === stagingEnvironmentName,
    ),
    'Railway staging environment does not match the manifest',
  )

  const manifestServices = manifest?.services
  assertObject(manifestServices, 'manifest.services')
  const expected = [
    {
      id: requiredString(railway.databaseServiceId, 'Railway database service ID'),
      name: 'scope-postgres',
    },
    ...requiredServices.map((key) => {
      const service = manifestServices[key]
      assertObject(service, `manifest.services.${key}`)
      return {
        id: requiredString(service.id, `${key} service ID`),
        name: requiredString(service.name, `${key} service name`),
      }
    }),
    {
      id: requiredString(railway.staging.routerServiceId, 'staging router service ID'),
      name: requiredString(railway.staging.routerServiceName, 'staging router service name'),
    },
  ]

  assert(Array.isArray(services), 'Railway service state must be an array')
  for (const service of expected) {
    assert(
      services.some(({ id, name }) => id === service.id && name === service.name),
      `Railway service ${service.name} does not match the manifest`,
    )
  }

  for (const key of ['apiDomain', 'cacheDomain', 'routerDomain', 'webDomain']) {
    const domain = requiredString(railway.staging[key], `staging ${key}`)
    assert.match(
      domain,
      /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/,
      `staging ${key} must be a hostname without a scheme or path`,
    )
  }

  return {
    apiReplicas,
    productionEnvironmentId,
    projectId,
    routerReplicas,
    stagingEnvironmentId,
    stagingEnvironmentName,
  }
}

export function verifyStagingTopology({ manifest, services }) {
  const railway = manifest?.railway
  assertObject(railway, 'manifest.railway')
  assertObject(railway.staging, 'manifest.railway.staging')
  assertObject(manifest?.services, 'manifest.services')
  assert(Array.isArray(services), 'Railway service state must be an array')

  const expected = [
    ['api', manifest.services.api?.id, railway.staging.apiReplicas],
    ['cache', manifest.services.cache?.id, 1],
    ['router', railway.staging.routerServiceId, railway.staging.routerReplicas],
    ['worker', manifest.services.worker?.id, 1],
  ]
  for (const [name, id, count] of expected) {
    const serviceId = requiredString(id, `${name} service ID`)
    const replicaCount = requiredPositiveInteger(count, `${name} replicas`)
    const service = services.find((candidate) => candidate.id === serviceId)
    const replicas = service?.replicas ?? {}
    assert(
      service?.status === 'SUCCESS' &&
        replicas.configured === replicaCount &&
        replicas.running === replicaCount &&
        (replicas.crashed ?? 0) === 0,
      `Staging ${name} must have exactly ${replicaCount} healthy replicas`,
    )
  }
}

function assertObject(value, name) {
  assert(value && typeof value === 'object' && !Array.isArray(value), `${name} is required`)
}

function requiredString(value, name) {
  assert(typeof value === 'string' && value.length > 0, `${name} is required`)
  return value
}

function requiredPositiveInteger(value, name) {
  assert(Number.isInteger(value) && value > 0, `${name} must be a positive integer`)
  return value
}

if (import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const input = {
    manifest: parseEnvironmentJson('SCOPE_DEPLOYMENT_MANIFEST_JSON'),
    services: parseEnvironmentJson('SCOPE_RAILWAY_SERVICES_JSON'),
    status: parseEnvironmentJson('SCOPE_RAILWAY_STATUS_JSON'),
  }
  const result = verifyStagingTarget(input)
  if (process.env.SCOPE_VERIFY_STAGING_TOPOLOGY === '1') verifyStagingTopology(input)
  process.stdout.write(`${JSON.stringify(result)}\n`)
}

function parseEnvironmentJson(name) {
  const value = process.env[name]
  assert(value, `${name} is required`)
  return JSON.parse(value)
}
