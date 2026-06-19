import { Button } from '@/components/ui/button'
import { authCookieName, createScopeShooAuth } from '@/lib/auth'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import {
  GitBranch,
  LoaderCircle,
  LogIn,
  LogOut,
  Moon,
  Sun,
} from 'lucide-react'
import { useState } from 'react'

type HomeState = {
  signedIn: boolean
}

type ThemeMode = 'dark' | 'light'

const loadHomeForRequest = createServerFn({ method: 'GET' }).handler(
  async (): Promise<HomeState> => {
    const { getCookie } = await import('@tanstack/react-start/server')
    return { signedIn: Boolean(getCookie(authCookieName)) }
  },
)

export const Route = createFileRoute('/')({
  loader: () => loadHomeForRequest(),
  component: ScopeHome,
})

function ScopeHome() {
  const home = Route.useLoaderData()
  const [signedIn, setSignedIn] = useState(home.signedIn)
  const [theme, setTheme] = useState<ThemeMode>('dark')

  function toggleTheme() {
    const nextTheme = theme === 'dark' ? 'light' : 'dark'
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex min-h-16 max-w-[980px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
              <GitBranch className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">Scope</div>
              <div className="truncate text-xs leading-4 text-muted-foreground">
                Repositories
              </div>
            </div>
          </div>

          <div className="flex min-w-0 items-center gap-2">
            <AuthControls signedIn={signedIn} setSignedIn={setSignedIn} />
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        </div>
      </header>

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="border-b border-border pb-6">
          <h1 className="text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
            Repositories
          </h1>
        </div>

        <div className="mt-8 border-y border-border">
          <div className="grid gap-2 py-10 text-sm sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
            <div className="min-w-0">
              <div className="font-medium leading-5">No repositories</div>
              <div className="mt-1 leading-5 text-muted-foreground">
                {signedIn
                  ? 'Repository creation is not available yet.'
                  : 'Sign in to start from an empty workspace.'}
              </div>
            </div>
            {!signedIn && (
              <Button size="sm" onClick={() => signIn()} type="button">
                <LogIn className="size-3.5" />
                <span>Sign in</span>
              </Button>
            )}
          </div>
        </div>
      </section>
    </main>
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

function AuthControls({
  signedIn,
  setSignedIn,
}: {
  signedIn: boolean
  setSignedIn: (signedIn: boolean) => void
}) {
  const [busy, setBusy] = useState(false)
  const title = signedIn ? 'Sign out' : 'Sign in with Shoo'

  async function toggleAuth() {
    setBusy(true)

    if (signedIn) {
      createScopeShooAuth().clearIdentity()
      await fetch('/auth/session', { method: 'DELETE' }).catch(() => undefined)
      setSignedIn(false)
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

async function signIn() {
  await createScopeShooAuth().startSignIn({ requestPii: true })
}

function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') {
    return
  }

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}
