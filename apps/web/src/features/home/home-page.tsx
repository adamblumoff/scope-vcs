import { homeFlashKey } from '@/api/client'
import { setupPushSecretKey, setupSecretKey } from '@/api/setup'
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
import { useEffect, useState } from 'react'
import { CreateRepoForm } from './create-repo-form'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import { RepoList } from './repo-list'

type ThemeMode = 'dark' | 'light'

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
  const [account, setAccount] = useState(home.account)
  const [createError, setCreateError] = useState<string | null>(null)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<RepoSummary | null>(null)
  const [flash, setFlash] = useState<string | null>(null)
  const [repositories, setRepositories] = useState(home.repositories)
  const [sessionError, setSessionError] = useState<string | null>(null)
  const [signedIn, setSignedIn] = useState(home.signedIn)
  const [theme, setTheme] = useState<ThemeMode>('dark')

  useEffect(() => {
    const message = window.sessionStorage.getItem(homeFlashKey)
    if (!message) {
      return
    }

    window.sessionStorage.removeItem(homeFlashKey)
    setFlash(message)
  }, [])

  useEffect(() => {
    setAccount(home.account)
    setRepositories(home.repositories)
    setSignedIn(home.signedIn)
  }, [home.account, home.repositories, home.signedIn])

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  async function createRepository(input: CreateRepoInput) {
    setCreateError(null)
    setDeleteError(null)
    try {
      const created = await createRepo(input)
      const repo = created.repo
      setRepositories((current) => [repo, ...current])
      if (created.setup.token?.secret) {
        storeSetupSecret(setupSecretKey(repo.id), created.setup.token.secret)
      }
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
      setCreateError(
        error instanceof Error ? error.message : 'repository creation failed',
      )
    }
  }

  async function deleteRepository(repo: RepoSummary) {
    setDeleteError(null)
    const deleted = await deleteRepo({
      owner: repo.owner_handle,
      repo: repo.name,
    })
    setRepositories((current) =>
      current.filter((candidate) => candidate.id !== deleted.id),
    )
    await router.invalidate()
    setDeleteTarget(null)
  }

  async function signOut() {
    setSessionError(null)
    let response: Response
    try {
      response = await fetch('/auth/session', { method: 'DELETE' })
    } catch (error) {
      setSessionError(error instanceof Error ? error.message : 'sign out failed')
      return
    }

    if (!response.ok) {
      setSessionError(`sign out failed: ${response.status}`)
      return
    }

    createScopeShooAuth().clearIdentity()
    setAccount(null)
    setRepositories([])
    setSignedIn(false)
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
          onDelete={(repo) => setDeleteTarget(repo)}
          onSignIn={signIn}
          repositories={repositories}
          signedIn={signedIn}
        />
      </section>

      {deleteTarget && (
        <DeleteRepositoryDialog
          onCancel={() => setDeleteTarget(null)}
          onConfirm={async (repo) => {
            try {
              await deleteRepository(repo)
            } catch (error) {
              setDeleteError(
                error instanceof Error ? error.message : 'repository deletion failed',
              )
              throw error
            }
          }}
          repo={deleteTarget}
        />
      )}
    </main>
  )
}

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
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
