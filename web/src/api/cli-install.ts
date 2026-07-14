import { getCliInstallConnection } from '@/api/client'
import type { CliInstallCommands } from '@/api/types'

export function buildCliInstallCommands(): CliInstallCommands {
  const baseUrl = getCliInstallConnection()
  return {
    posix: `curl -fsSL ${baseUrl}/install.sh | sh`,
    windows: `irm ${baseUrl}/install.ps1 | iex`,
  }
}
