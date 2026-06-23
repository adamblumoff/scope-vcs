import { spawnSync } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(webRoot, '..')
const apiManifest = path.join(repoRoot, 'api', 'Cargo.toml')
const generatedTypesPath = path.join(webRoot, 'src', 'api', 'types.generated.ts')
const tempDir = await mkdtemp(path.join(os.tmpdir(), 'scope-api-types-'))
const tempTypesPath = path.join(tempDir, 'types.generated.ts')

try {
  const result = spawnSync(
    'cargo',
    ['test', '--manifest-path', apiManifest, 'export_api_types', '--', '--ignored'],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        SCOPE_API_TS_EXPORT_PATH: tempTypesPath,
      },
      stdio: 'inherit',
    },
  )

  if (result.status !== 0) {
    process.exit(result.status ?? 1)
  }

  const [checkedIn, generated] = await Promise.all([
    readFile(generatedTypesPath, 'utf8'),
    readFile(tempTypesPath, 'utf8'),
  ])

  if (normalizeLineEndings(checkedIn) !== normalizeLineEndings(generated)) {
    console.error('Generated API types are stale.')
    console.error(
      'Run `cargo test --manifest-path api/Cargo.toml export_api_types -- --ignored` from the repo root.',
    )
    process.exit(1)
  }
} finally {
  await rm(tempDir, { force: true, recursive: true })
}

function normalizeLineEndings(source) {
  return source.replace(/\r\n?/g, '\n')
}
