import type { RepoParams, RepoSetupView } from '@/api/types'
import {
  setupPushSecretSnapshot,
  storeSetupPushSecret,
} from '@/lib/setup-push-secret'
import { useCallback, useSyncExternalStore } from 'react'
import type { SetupPageAction } from './setup-page-state'

export function useRegenerateSetupToken({
  dispatch,
  initialSetup,
  params,
  regenerateToken,
}: {
  dispatch: (action: SetupPageAction) => void
  initialSetup: RepoSetupView
  params: RepoParams
  regenerateToken: (params: RepoParams) => Promise<RepoSetupView>
}) {
  return useCallback(async () => {
    dispatch({ type: 'tokenStarted' })
    try {
      const next = await regenerateToken(params)
      const pushTokenSecret = next.push_token?.secret ?? null
      storeSetupPushSecret(next.repo.id, pushTokenSecret)
      dispatch({
        baseSetup: initialSetup,
        pushTokenSecret,
        setup: next,
        type: 'tokenSucceeded',
      })
    } catch (tokenError) {
      dispatch({
        message:
          tokenError instanceof Error
            ? tokenError.message
            : 'setup command update failed',
        type: 'tokenFailed',
      })
    }
  }, [dispatch, initialSetup, params, regenerateToken])
}

export function useSetupPushSecret(repoId: string) {
  return useSyncExternalStore(
    subscribeSetupPushSecret,
    () => setupPushSecretSnapshot(repoId),
    getServerSetupPushSecretSnapshot,
  )
}

function subscribeSetupPushSecret() {
  return () => {}
}

function getServerSetupPushSecretSnapshot() {
  return null
}
