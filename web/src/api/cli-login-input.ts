export type CompleteCliLoginInput = {
  code: string
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

export function normalizeCliLoginCode(value: string) {
  return value.trim().replaceAll('-', '').replace(/\s/g, '').toUpperCase()
}
