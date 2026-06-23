import type {
  CreateRepoInput,
  CreateRepoResponse,
  HomeState,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { createScopeShooAuth } from '@/lib/auth'
import { useHomeFlash } from '@/lib/home-flash'
import { storeSetupPushSecret } from '@/lib/setup-push-secret'
import { useNavigate, useRouter } from '@tanstack/react-router'
import {
  CheckCircle2,
  LoaderCircle,
  LogOut,
  Moon,
  Sun,
} from 'lucide-react'
import { useReducer, useState } from 'react'
import { CreateRepoForm } from './create-repo-form'
import {
  type ThemeMode,
  activeHomeState,
  homePageReducer,
  initialHomePageState,
  nextThemeMode,
} from './home-page-state'
import { RepoList } from './repo-list'

export function HomePage({
  createRepo,
  home,
}: {
  createRepo: (input: CreateRepoInput) => Promise<CreateRepoResponse>
  home: HomeState
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const [state, dispatch] = useReducer(
    homePageReducer,
    initialHomePageState,
  )
  const flash = useHomeFlash()
  const activeHome = activeHomeState(home, state)
  const { account, repositories, signedIn } = activeHome
  const { createError, sessionError, theme } = state

  function toggleTheme() {
    const nextTheme = nextThemeMode(theme)
    dispatch({ theme: nextTheme, type: 'themeChanged' })
    applyTheme(nextTheme)
  }

  async function createRepository(input: CreateRepoInput) {
    dispatch({ type: 'createStarted' })
    try {
      const created = await createRepo(input)
      const repo = created.repo
      dispatch({
        account,
        baseHome: home,
        repositories,
        repo,
        signedIn,
        type: 'repositoryCreated',
      })
      storeSetupPushSecret(repo.id, created.setup.push_token?.secret ?? null)
      await router.invalidate()
      await navigate({
        to: '/repos/$owner/$repo/setup',
        params: { owner: repo.owner_handle, repo: repo.name },
      })
    } catch (error) {
      dispatch({
        message:
          error instanceof Error ? error.message : 'repository creation failed',
        type: 'createFailed',
      })
    }
  }

  async function signOut() {
    dispatch({ type: 'sessionStarted' })
    let response: Response
    try {
      response = await fetch('/auth/session', { method: 'DELETE' })
    } catch (error) {
      dispatch({
        message: error instanceof Error ? error.message : 'sign out failed',
        type: 'sessionFailed',
      })
      return
    }

    if (!response.ok) {
      dispatch({
        message: `sign out failed: ${response.status}`,
        type: 'sessionFailed',
      })
      return
    }

    createScopeShooAuth().clearIdentity()
    dispatch({ baseHome: home, type: 'signedOut' })
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
          <PageErrorAlert title="Repositories unavailable">
            {home.error}
          </PageErrorAlert>
        )}

        {createError && (
          <PageErrorAlert title="Repository creation failed">
            {createError}
          </PageErrorAlert>
        )}

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        {sessionError && (
          <PageErrorAlert title="Session update failed">
            {sessionError}
          </PageErrorAlert>
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

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
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

function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') {
    return
  }

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}
