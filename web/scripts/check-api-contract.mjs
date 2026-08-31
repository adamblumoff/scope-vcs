import { spawnSync } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const tempDir = await mkdtemp(path.join(os.tmpdir(), 'scope-api-contract-'))
const artifacts = [
  'types.generated.ts',
  'schemas.generated.json',
  'validators.generated.ts',
]

try {
  const result = spawnSync(
    process.execPath,
    [path.join(webRoot, 'scripts', 'generate-api-contract.mjs')],
    {
      cwd: webRoot,
      env: {
        ...process.env,
        SCOPE_API_SCHEMA_EXPORT_PATH: path.join(tempDir, artifacts[1]),
        SCOPE_API_TS_EXPORT_PATH: path.join(tempDir, artifacts[0]),
        SCOPE_API_VALIDATOR_EXPORT_PATH: path.join(tempDir, artifacts[2]),
      },
      stdio: 'inherit',
    },
  )

  if (result.status !== 0) process.exit(result.status ?? 1)

  for (const artifact of artifacts) {
    const [checkedIn, generated] = await Promise.all([
      readFile(path.join(webRoot, 'src', 'api', artifact), 'utf8'),
      readFile(path.join(tempDir, artifact), 'utf8'),
    ])
    if (normalizeLineEndings(checkedIn) !== normalizeLineEndings(generated)) {
      console.error(`Generated API contract artifact is stale: ${artifact}`)
      console.error('Run `pnpm generate:api-contract` from web/.')
      process.exit(1)
    }
  }
} finally {
  await rm(tempDir, { force: true, recursive: true })
}

function normalizeLineEndings(source) {
  return source.replace(/\r\n?/g, '\n')
}
