import { homeFlashKey } from '@/api/client'
import { setupPushSecretKey } from '@/api/setup'
import type {
  CreateRepoInput,
  CreateRepoResponse,
  HomeState,
  RepoSummary,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { createScopeShooAuth } from '@/lib/auth'
import { useNavigate, useRouter } from '@tanstack/react-router'
import {
  AlertCircle,
  CheckCircle2,
  LoaderCircle,
  LogOut,
  Moon,
  Sun,
} from 'lucide-react'
import { useReducer, useState, useSyncExternalStore } from 'react'
import { CreateRepoForm } from './create-repo-form'
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
  sessionError: string | null
}

type HomeUiAction =
  | { type: 'createFailed'; message: string }
  | { type: 'createStarted' }
  | { type: 'sessionFailed'; message: string }
  | { type: 'sessionStarted' }

const initialHomeUiState: HomeUiState = {
  createError: null,
  sessionError: null,
}

export function HomePage({
  createRepo,
  home,
}: {
  createRepo: (input: CreateRepoInput) => Promise<CreateRepoResponse>
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
  const { createError, sessionError } = uiState

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
            {signedIn && <AuthControls signOut={signOut} />}
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        }
        homeLink={false}
        subtitle={account?.user?.handle ?? 'Repositories'}
      />

      <PageContent>
        <PageHeader
          actions={
            signedIn
              ? () => <CreateRepoForm onCreate={createRepository} />
              : undefined
          }
          description={account?.identity?.email ?? account?.identity?.pairwise_sub}
          title="Repositories"
        />

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

        {sessionError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Session update failed</AlertTitle>
            <AlertDescription>{sessionError}</AlertDescription>
          </Alert>
        )}

        <RepoList
          onSignIn={signIn}
          repositories={repositories}
          signedIn={signedIn}
        />
      </PageContent>
    </main>
  )
}

function homeUiReducer(state: HomeUiState, action: HomeUiAction): HomeUiState {
  switch (action.type) {
    case 'createStarted':
      return { ...state, createError: null }
    case 'createFailed':
      return { ...state, createError: action.message }
    case 'sessionStarted':
      return { ...state, sessionError: null }
    case 'sessionFailed':
      return { ...state, sessionError: action.message }
  }
}

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
}

type HomeFlashSnapshot = {
  value: string | null | undefined
}

function useHomeFlash() {
  const [snapshot] = useState<HomeFlashSnapshot>(() => ({ value: undefined }))
  return useSyncExternalStore(
    subscribeHomeFlash,
    () => getHomeFlashSnapshot(snapshot),
    getServerHomeFlashSnapshot,
  )
}

function subscribeHomeFlash() {
  return () => {}
}

function getHomeFlashSnapshot(snapshot: HomeFlashSnapshot) {
  if (snapshot.value !== undefined) {
    return snapshot.value
  }

  if (typeof window === 'undefined') {
    return null
  }

  snapshot.value = window.sessionStorage.getItem(homeFlashKey)
  if (snapshot.value) {
    window.sessionStorage.removeItem(homeFlashKey)
  }

  return snapshot.value
}

function getServerHomeFlashSnapshot() {
  return null
}

function AuthControls({
  signOut,
}: {
  signOut: () => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  const title = 'Sign out'

  async function signOutUser() {
    setBusy(true)
    try {
      await signOut()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Button
      aria-label={title}
      disabled={busy}
      onClick={() => void signOutUser()}
      size="sm"
      title={title}
      type="button"
      variant="secondary"
    >
      {busy ? (
        <LoaderCircle className="size-3.5 animate-spin" />
      ) : (
        <LogOut className="size-3.5" />
      )}
      {!busy && <span className="hidden sm:inline">Sign out</span>}
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
