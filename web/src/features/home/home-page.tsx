import { homeFlashKey } from '@/api/client'
import { setupPushSecretKey } from '@/api/setup'
import type {
  CreateRepoInput,
  CreateRepoResponse,
  DeleteRepoInput,
  DeleteRepoResponse,
  HomeState,
  RepoSummary,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { createScopeShooAuth } from '@/lib/auth'
import { useNavigate, useRouter } from '@tanstack/react-router'
import {
  AlertCircle,
  CheckCircle2,
  LoaderCircle,
  LogIn,
  LogOut,
  Moon,
  Sun,
} from 'lucide-react'
import { useReducer, useState, useSyncExternalStore } from 'react'
import { CreateRepoForm } from './create-repo-form'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import { RepoList } from './repo-list'

type ThemeMode = 'dark' | 'light'

type HomeOverride = {
  account: HomeState['account']
  baseHome: HomeState
  repositories: RepoSummary[]
  signedIn: boolean
}

type HomeUiState = {
  createError: string | null
  deleteError: string | null
  deleteTarget: RepoSummary | null
  sessionError: string | null
}

type HomeUiAction =
  | { type: 'createFailed'; message: string }
  | { type: 'createStarted' }
  | { type: 'deleteFailed'; message: string }
  | { type: 'deleteStarted' }
  | { type: 'deleteSucceeded' }
  | { type: 'deleteTargetChanged'; repo: RepoSummary | null }
  | { type: 'sessionFailed'; message: string }
  | { type: 'sessionStarted' }

const initialHomeUiState: HomeUiState = {
  createError: null,
  deleteError: null,
  deleteTarget: null,
  sessionError: null,
}

export function HomePage({
  createRepo,
  deleteRepo,
  home,
}: {
  createRepo: (input: CreateRepoInput) => Promise<CreateRepoResponse>
  deleteRepo: (input: DeleteRepoInput) => Promise<DeleteRepoResponse>
  home: HomeState
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const [uiState, dispatchUi] = useReducer(
    homeUiReducer,
    initialHomeUiState,
  )
  const [homeOverride, setHomeOverride] = useState<HomeOverride | null>(null)
  const flash = useHomeFlash()
  const [theme, setTheme] = useState<ThemeMode>('dark')
  const activeHome = homeOverride?.baseHome === home ? homeOverride : home
  const { account, repositories, signedIn } = activeHome
  const { createError, deleteError, deleteTarget, sessionError } = uiState

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  async function createRepository(input: CreateRepoInput) {
    dispatchUi({ type: 'createStarted' })
    try {
      const created = await createRepo(input)
      const repo = created.repo
      setHomeOverride({
        account,
        baseHome: home,
        repositories: [repo, ...repositories],
        signedIn,
      })
      if (created.setup.push_token?.secret) {
        storeSetupSecret(
          setupPushSecretKey(repo.id),
          created.setup.push_token.secret,
        )
      }
      await router.invalidate()
      await navigate({
        to: '/repos/$owner/$repo/setup',
        params: { owner: repo.owner_handle, repo: repo.name },
      })
    } catch (error) {
      dispatchUi({
        message:
          error instanceof Error ? error.message : 'repository creation failed',
        type: 'createFailed',
      })
    }
  }

  async function deleteRepository(repo: RepoSummary) {
    dispatchUi({ type: 'deleteStarted' })
    const deleted = await deleteRepo({
      owner: repo.owner_handle,
      repo: repo.name,
    })
    setHomeOverride({
      account,
      baseHome: home,
      repositories: repositories.filter((candidate) => candidate.id !== deleted.id),
      signedIn,
    })
    await router.invalidate()
    dispatchUi({ type: 'deleteSucceeded' })
  }

  async function signOut() {
    dispatchUi({ type: 'sessionStarted' })
    let response: Response
    try {
      response = await fetch('/auth/session', { method: 'DELETE' })
    } catch (error) {
      dispatchUi({
        message: error instanceof Error ? error.message : 'sign out failed',
        type: 'sessionFailed',
      })
      return
    }

    if (!response.ok) {
      dispatchUi({
        message: `sign out failed: ${response.status}`,
        type: 'sessionFailed',
      })
      return
    }

    createScopeShooAuth().clearIdentity()
    setHomeOverride({
      account: null,
      baseHome: home,
      repositories: [],
      signedIn: false,
    })
    await router.invalidate()
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        action={
          <div className="flex min-w-0 items-center gap-2">
            <AuthControls signedIn={signedIn} signOut={signOut} />
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        }
        homeLink={false}
        subtitle={account?.user?.handle ?? 'Repositories'}
      />

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <h1 className="text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              Repositories
            </h1>
            {account?.identity && (
              <div className="mt-2 truncate text-sm leading-5 text-muted-foreground">
                {account.identity.email ?? account.identity.pairwise_sub}
              </div>
            )}
          </div>
          {signedIn && <CreateRepoForm onCreate={createRepository} />}
        </div>

        {home.error && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repositories unavailable</AlertTitle>
            <AlertDescription>{home.error}</AlertDescription>
          </Alert>
        )}

        {createError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repository creation failed</AlertTitle>
            <AlertDescription>{createError}</AlertDescription>
          </Alert>
        )}

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        {deleteError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repository deletion failed</AlertTitle>
            <AlertDescription>{deleteError}</AlertDescription>
          </Alert>
        )}

        {sessionError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Session update failed</AlertTitle>
            <AlertDescription>{sessionError}</AlertDescription>
          </Alert>
        )}

        <RepoList
          onDelete={(repo) =>
            dispatchUi({ repo, type: 'deleteTargetChanged' })
          }
          onSignIn={signIn}
          repositories={repositories}
          signedIn={signedIn}
        />
      </section>

      {deleteTarget && (
        <DeleteRepositoryDialog
          onCancel={() =>
            dispatchUi({ repo: null, type: 'deleteTargetChanged' })
          }
          onConfirm={async (repo) => {
            try {
              await deleteRepository(repo)
            } catch (error) {
              dispatchUi({
                message:
                  error instanceof Error
                    ? error.message
                    : 'repository deletion failed',
                type: 'deleteFailed',
              })
              throw error
            }
          }}
          repo={deleteTarget}
        />
      )}
    </main>
  )
}

function homeUiReducer(state: HomeUiState, action: HomeUiAction): HomeUiState {
  switch (action.type) {
    case 'createStarted':
      return { ...state, createError: null, deleteError: null }
    case 'createFailed':
      return { ...state, createError: action.message }
    case 'deleteStarted':
      return { ...state, deleteError: null }
    case 'deleteFailed':
      return { ...state, deleteError: action.message }
    case 'deleteSucceeded':
      return { ...state, deleteTarget: null }
    case 'deleteTargetChanged':
      return { ...state, deleteTarget: action.repo }
    case 'sessionStarted':
      return { ...state, sessionError: null }
    case 'sessionFailed':
      return { ...state, sessionError: action.message }
  }
}

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
}

let cachedHomeFlash: string | null | undefined

function useHomeFlash() {
  return useSyncExternalStore(
    subscribeHomeFlash,
    getHomeFlashSnapshot,
    getServerHomeFlashSnapshot,
  )
}

function subscribeHomeFlash() {
  return () => {}
}

function getHomeFlashSnapshot() {
  if (cachedHomeFlash !== undefined) {
    return cachedHomeFlash
  }

  if (typeof window === 'undefined') {
    return null
  }

  cachedHomeFlash = window.sessionStorage.getItem(homeFlashKey)
  if (cachedHomeFlash) {
    window.sessionStorage.removeItem(homeFlashKey)
  }

  return cachedHomeFlash
}

function getServerHomeFlashSnapshot() {
  return null
}

function AuthControls({
  signedIn,
  signOut,
}: {
  signedIn: boolean
  signOut: () => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  const title = signedIn ? 'Sign out' : 'Sign in with Shoo'

  async function toggleAuth() {
    setBusy(true)

    if (signedIn) {
      await signOut()
      setBusy(false)
      return
    }

    try {
      await signIn()
    } catch {
      setBusy(false)
    }
  }

  return (
    <Button
      aria-label={title}
      disabled={busy}
      onClick={() => void toggleAuth()}
      size="sm"
      title={title}
      type="button"
      variant={signedIn ? 'secondary' : 'default'}
    >
      {busy ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : signedIn ? (
        <LogOut className="size-3.5" />
      ) : (
        <LogIn className="size-3.5" />
      )}
      {!busy && (
        <span className="hidden sm:inline">{signedIn ? 'Sign out' : 'Sign in'}</span>
      )}
    </Button>
  )
}

function ThemeToggle({
  theme,
  toggleTheme,
}: {
  theme: ThemeMode
  toggleTheme: () => void
}) {
  const nextTheme = theme === 'dark' ? 'Light' : 'Dark'

  return (
    <Button
      aria-label={`Switch to ${nextTheme} mode`}
      onClick={toggleTheme}
      size="icon-sm"
      title={`Switch to ${nextTheme} mode`}
      type="button"
      variant="secondary"
    >
      {theme === 'dark' ? (
        <Sun className="size-3.5" />
      ) : (
        <Moon className="size-3.5" />
      )}
    </Button>
  )
}

function storeSetupSecret(key: string, secret: string) {
  if (typeof window === 'undefined') {
    return
  }

  window.sessionStorage.setItem(key, secret)
}

function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') {
    return
  }

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}
