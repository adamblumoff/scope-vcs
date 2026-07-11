export type CompleteCliLoginInput = {
  code: string
}

export type CompleteBrowserCliLoginInput = {
  requestId: string
}

export type RevokeCliSessionInput = {
  sessionId: string
}

export function parseCompleteCliLoginInput(
  input: unknown,
): CompleteCliLoginInput {
  const data = input as Partial<CompleteCliLoginInput> | null
  const code = typeof data?.code === 'string' ? normalizeCliLoginCode(data.code) : ''
  if (!code) {
    throw new Error('CLI login code is required.')
  }

  return { code }
}

export function parseCompleteBrowserCliLoginInput(
  input: unknown,
): CompleteBrowserCliLoginInput {
  const data = input as Partial<CompleteBrowserCliLoginInput> | null
  const requestId = typeof data?.requestId === 'string' ? data.requestId.trim() : ''
  if (!requestId.startsWith('cli_browser_')) {
    throw new Error('CLI browser login request is invalid.')
  }

  return { requestId }
}

export function parseRevokeCliSessionInput(
  input: unknown,
): RevokeCliSessionInput {
  const data = input as Partial<RevokeCliSessionInput> | null
  const sessionId = typeof data?.sessionId === 'string' ? data.sessionId.trim() : ''
  if (!sessionId.startsWith('cli_sess_')) {
    throw new Error('CLI session is invalid.')
  }

  return { sessionId }
}

function normalizeCliLoginCode(value: string) {
  return value.trim().replaceAll('-', '').replace(/\s/g, '').toUpperCase()
}
