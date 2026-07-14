import type { CliPlatform } from '@/api/types'

export function detectCliPlatform(userAgent: string | undefined): CliPlatform {
  return userAgent && /windows|win32|win64/i.test(userAgent)
    ? 'windows'
    : 'posix'
}
