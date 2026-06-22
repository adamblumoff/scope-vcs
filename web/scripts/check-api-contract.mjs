import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(webRoot, '..')
const rustPath = path.join(repoRoot, 'api', 'src', 'http', 'responses.rs')
const tsPath = path.join(webRoot, 'src', 'api', 'types.ts')

const [rustSource, tsSource] = await Promise.all([
  readFile(rustPath, 'utf8'),
  readFile(tsPath, 'utf8'),
])

const rustStructs = parseRustStructs(rustSource)
const tsTypes = parseTsObjectTypes(tsSource)

const contracts = [
  ['AccountSessionResponse', 'AccountSession'],
  ['UserResponse', 'User'],
  ['SessionResponse', 'RepoSession'],
  ['SessionIdentity', 'SessionIdentity'],
  ['SessionRepo', 'SessionRepo'],
  ['SessionCapabilities', 'RepoCapabilities'],
  ['RepoSummaryResponse', 'RepoSummary'],
  ['CreateRepoResponse', 'CreateRepoResponse'],
  ['DeleteRepoResponse', 'DeleteRepoResponse'],
  ['RepoSetupResponse', 'RepoSetup'],
  ['RepoGitCredentialResponse', 'RepoGitCredential'],
  ['FirstPushTokenResponse', 'FirstPushToken'],
  ['GitPushTokenResponse', 'GitPushToken'],
  ['RepoFileResponse', 'RepoFile'],
  ['PendingImportReviewResponse', 'PendingImportPayload'],
  ['StagedUpdateResponse', 'StagedUpdate'],
  ['StagedFileResponse', 'StagedFile'],
]

const tsNameByRustName = new Map(contracts)
tsNameByRustName.set('FirstPushTokenStatus', 'TokenStatus')
const failures = []

for (const [rustName, tsName] of contracts) {
  const rustFields = rustStructs.get(rustName)
  const tsFields = tsTypes.get(tsName)

  if (!rustFields) {
    failures.push(`Missing Rust struct ${rustName}`)
    continue
  }

  if (!tsFields) {
    failures.push(`Missing TypeScript type ${tsName}`)
    continue
  }

  const rustFieldNames = [...rustFields.keys()].sort()
  const tsFieldNames = [...tsFields.keys()].sort()
  if (rustFieldNames.join(',') !== tsFieldNames.join(',')) {
    failures.push(
      `${rustName} -> ${tsName} fields differ\n` +
        `  Rust: ${rustFieldNames.join(', ') || '(none)'}\n` +
        `  TS:   ${tsFieldNames.join(', ') || '(none)'}`,
    )
    continue
  }

  for (const fieldName of rustFieldNames) {
    const expectedType = tsTypeForRust(rustFields.get(fieldName))
    const actualType = normalizeTsType(tsFields.get(fieldName))
    if (actualType !== expectedType) {
      failures.push(
        `${rustName}.${fieldName} -> ${tsName}.${fieldName} type differs: ` +
          `Rust expects ${expectedType}, TS has ${actualType}`,
      )
    }
  }
}

if (failures.length > 0) {
  console.error('API contract drift detected:')
  for (const failure of failures) {
    console.error(`\n${failure}`)
  }
  process.exit(1)
}

function parseRustStructs(source) {
  const structs = new Map()
  const structPattern = /pub\(crate\)\s+struct\s+(\w+)\s*\{([\s\S]*?)\n\}/g

  for (const match of source.matchAll(structPattern)) {
    const [, name, body] = match
    const fields = new Map()

    for (const line of body.split('\n')) {
      const field = line.trim().match(/^pub\(crate\)\s+(\w+):\s+(.+),$/)
      if (field) {
        fields.set(field[1], field[2].trim())
      }
    }

    structs.set(name, fields)
  }

  return structs
}

function parseTsObjectTypes(source) {
  const types = new Map()
  const typePattern = /export\s+type\s+(\w+)\s*=\s*\{/g
  let match

  while ((match = typePattern.exec(source))) {
    const name = match[1]
    const bodyStart = source.indexOf('{', match.index)
    const bodyEnd = findMatchingBrace(source, bodyStart)
    const body = source.slice(bodyStart + 1, bodyEnd)
    const fields = new Map()

    for (const line of body.split('\n')) {
      const field = line.trim().match(/^(\w+):\s+(.+)$/)
      if (field) {
        fields.set(field[1], field[2].trim())
      }
    }

    types.set(name, fields)
    typePattern.lastIndex = bodyEnd + 1
  }

  return types
}

function findMatchingBrace(source, start) {
  let depth = 0

  for (let index = start; index < source.length; index += 1) {
    const character = source[index]
    if (character === '{') {
      depth += 1
    } else if (character === '}') {
      depth -= 1
      if (depth === 0) {
        return index
      }
    }
  }

  throw new Error('Unclosed TypeScript object type')
}

function tsTypeForRust(type) {
  const trimmed = type.trim()
  const option = unwrapGeneric(trimmed, 'Option')
  if (option) {
    return `${tsTypeForRust(option)} | null`
  }

  const vector = unwrapGeneric(trimmed, 'Vec')
  if (vector) {
    return `${tsTypeForRust(vector)}[]`
  }

  if (trimmed === 'String' || trimmed === "&'static str" || trimmed === 'ScopePath') {
    return 'string'
  }

  if (trimmed === 'bool') {
    return 'boolean'
  }

  if (trimmed === 'u64') {
    return 'number'
  }

  return tsNameByRustName.get(trimmed) ?? trimmed
}

function unwrapGeneric(type, genericName) {
  const prefix = `${genericName}<`
  if (!type.startsWith(prefix) || !type.endsWith('>')) {
    return null
  }

  return type.slice(prefix.length, -1).trim()
}

function normalizeTsType(type) {
  const cleaned = type.replace(/\s+/g, ' ').replace(/;$/, '').trim()
  const array = cleaned.match(/^(.+)\[\]$/)
  if (array) {
    return `${normalizeTsType(array[1])}[]`
  }

  const union = cleaned.split('|').map((part) => part.trim())
  if (union.length > 1) {
    return union
      .map((part) => normalizeTsType(part))
      .sort((left, right) =>
        left === 'null' ? 1 : right === 'null' ? -1 : left.localeCompare(right),
      )
      .join(' | ')
  }

  return cleaned
}
