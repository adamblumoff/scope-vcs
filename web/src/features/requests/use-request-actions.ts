import { useCallback, useState } from 'react'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'

export type RequestActionController = {
  error: string | null
  pending: RequestActionCommand['action'] | null
  run: (command: RequestActionCommand) => Promise<boolean>
}

export function useRequestActions(
  perform: (command: RequestActionCommand) => Promise<RequestActionResult>,
): RequestActionController {
  const [pending, setPending] = useState<RequestActionCommand['action'] | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = useCallback(async (command: RequestActionCommand) => {
    setPending(command.action)
    setError(null)
    try {
      const result = await perform(command)
      if (result.synchronizationError) setError(result.synchronizationError)
      return true
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : 'The request could not be updated.',
      )
      return false
    } finally {
      setPending(null)
    }
  }, [perform])

  return { error, pending, run }
}
