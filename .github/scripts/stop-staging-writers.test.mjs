import assert from 'node:assert/strict'
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { afterEach, test } from 'node:test'

const temporaryDirectories = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { force: true, recursive: true })))
})

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'scope-staging-stop-'))
  temporaryDirectories.push(root)
  const manifest = join(root, 'manifest.json')
  const removals = join(root, 'removals.jsonl')
  await writeFile(manifest, JSON.stringify({
    railway: {
      environmentId: 'production',
      projectId: 'project',
      staging: { environmentId: 'staging' },
    },
    services: {
      api: { id: 'api' },
      cache: { id: 'cache' },
      worker: { id: 'worker' },
    },
  }))
  await writeFile(join(root, 'railway'), `#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == *"deployment list"* ]]; then
  if [[ "$*" == *"--service api"* ]]; then
    printf '%s\\n' '[{"id":"api-deployment","status":"SUCCESS"}]'
  elif [[ "$*" == *"--service cache"* ]]; then
    printf '%s\\n' '[{"id":"cache-deployment","status":"SUCCESS"}]'
  else
    printf '%s\\n' '[{"id":"worker-deployment","status":"SUCCESS"}]'
  fi
elif [[ "$*" == *"service list"* ]]; then
  printf '%s\\n' '[{"id":"api","replicas":{"running":0,"crashed":0}},{"id":"cache","replicas":{"running":0,"crashed":0}},{"id":"worker","replicas":{"running":0,"crashed":0}}]'
else
  exit 2
fi
`)
  await writeFile(join(root, 'curl'), `#!/usr/bin/env bash
set -euo pipefail
while [[ "$#" -gt 0 ]]; do
  if [[ "$1" == "--data-binary" ]]; then
    printf '%s\\n' "$2" >> "$SCOPE_TEST_REMOVALS"
    break
  fi
  shift
done
printf '%s\\n' '{"data":{"deploymentRemove":true}}'
`)
  await chmod(join(root, 'railway'), 0o755)
  await chmod(join(root, 'curl'), 0o755)
  return { manifest, removals, root }
}

test('stops only the reviewed staging metadata-writer deployments', async () => {
  const { manifest, removals, root } = await fixture()
  const result = spawnSync('bash', ['.github/scripts/stop-staging-writers.sh'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${root}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: 'account-token',
      RAILWAY_TOKEN: '',
      SCOPE_DEPLOYMENT_MANIFEST: manifest,
      SCOPE_TEST_REMOVALS: removals,
    },
  })

  assert.equal(result.status, 0, result.stderr)
  const requests = (await readFile(removals, 'utf8')).trim().split('\n').map(JSON.parse)
  assert.deepEqual(requests.map(({ variables }) => variables.id), [
    'api-deployment',
    'worker-deployment',
    'cache-deployment',
  ])
})

test('rejects mixed Railway token privileges before making requests', async () => {
  const { manifest, root } = await fixture()
  const result = spawnSync('bash', ['.github/scripts/stop-staging-writers.sh'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${root}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: 'account-token',
      RAILWAY_TOKEN: 'project-token',
      SCOPE_DEPLOYMENT_MANIFEST: manifest,
    },
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /requires only RAILWAY_API_TOKEN/)
})
