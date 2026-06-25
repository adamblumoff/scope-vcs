export { HttpError, authHeaders, loadJson, stripTrailingSlash } from './http'
import { stripTrailingSlash } from './http'

const localApiBase = 'http://localhost:8080'
const localCliInstallBase = 'http://localhost:8787'
const cliInstallUrlEnv = 'SCOPE_CLI_INSTALL_URL'
const internalApiUrlEnv = 'SCOPE_API_INTERNAL_URL'
const publicApiUrlEnv = 'SCOPE_API_PUBLIC_URL'

export async function readRequestAuthToken() {
  const { auth } = await import('@clerk/tanstack-react-start/server')
  const { getToken } = await auth()
  return getToken()
}

export function getApiConnection(action = 'loading repositories') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getApiMutationConnection(action = 'changing repository state') {
  return configuredApiConnection(internalApiUrlEnv, action)
}

export function getPublicApiConnection(action = 'building public API URL') {
  return configuredApiConnection(publicApiUrlEnv, action)
}

export function getCliInstallConnection(action = 'building CLI install command') {
  return configuredConnection(cliInstallUrlEnv, localCliInstallBase, action)
}

function configuredApiConnection(envName: string, action: string) {
  return configuredConnection(envName, localApiBase, action)
}

function configuredConnection(envName: string, fallback: string, action: string) {
  const envBase = runtimeEnv(envName)
  if (envBase) {
    return stripTrailingSlash(envBase)
  }

  if (import.meta.env.DEV) {
    return fallback
  }

  throw new Error(`Set ${envName} before ${action}.`)
}

function runtimeEnv(name: string) {
  if (typeof process === 'undefined') {
    return undefined
  }

  return process.env[name]?.trim()
}

