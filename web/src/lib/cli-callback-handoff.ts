export const CLI_CALLBACK_FALLBACK_DELAY_MS = 3000

const CLI_CALLBACK_PATH = '/scope-cli-callback'
const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]'])

export function parseCliCallbackHandoffUrl(value: string): string {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    throw new Error('CLI authorization callback was invalid.')
  }

  if (
    url.protocol !== 'http:' ||
    !url.port ||
    url.pathname !== CLI_CALLBACK_PATH ||
    !LOOPBACK_HOSTS.has(url.hostname)
  ) {
    throw new Error('CLI authorization callback was invalid.')
  }

  return url.toString()
}
