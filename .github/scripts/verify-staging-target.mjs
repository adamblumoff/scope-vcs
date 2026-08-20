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
  ]

  assert(Array.isArray(services), 'Railway service state must be an array')
  for (const service of expected) {
    assert(
      services.some(({ id, name }) => id === service.id && name === service.name),
      `Railway service ${service.name} does not match the manifest`,
    )
  }

  for (const key of ['apiDomain', 'cacheDomain', 'webDomain']) {
    const domain = requiredString(railway.staging[key], `staging ${key}`)
    assert.match(
      domain,
      /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/,
      `staging ${key} must be a hostname without a scheme or path`,
    )
  }

  return {
    productionEnvironmentId,
    projectId,
    stagingEnvironmentId,
    stagingEnvironmentName,
  }
}

function assertObject(value, name) {
  assert(value && typeof value === 'object' && !Array.isArray(value), `${name} is required`)
}

function requiredString(value, name) {
  assert(typeof value === 'string' && value.length > 0, `${name} is required`)
  return value
}

if (import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const result = verifyStagingTarget({
    manifest: parseEnvironmentJson('SCOPE_DEPLOYMENT_MANIFEST_JSON'),
    services: parseEnvironmentJson('SCOPE_RAILWAY_SERVICES_JSON'),
    status: parseEnvironmentJson('SCOPE_RAILWAY_STATUS_JSON'),
  })
  process.stdout.write(`${JSON.stringify(result)}\n`)
}

function parseEnvironmentJson(name) {
  const value = process.env[name]
  assert(value, `${name} is required`)
  return JSON.parse(value)
}
