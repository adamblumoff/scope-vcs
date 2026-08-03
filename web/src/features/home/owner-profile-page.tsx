import type { ProfileState } from '@/api/types'
import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useHomeFlash } from '@/lib/home-flash'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { CheckCircle2, KeyRound, Moon, Sun } from 'lucide-react'
import { useSyncExternalStore } from 'react'
import { RepoList } from './repo-list'

type ThemeMode = 'dark' | 'light'

const THEME_STORAGE_KEY = 'scope-theme'
const THEME_CHANGE_EVENT = 'scope-theme-change'

export function OwnerProfilePage({ state }: { state: ProfileState }) {
  const theme = useSyncExternalStore(
    subscribeToTheme,
    readBrowserTheme,
    readServerTheme,
  )
  const flash = useHomeFlash()
  const { account, profile } = state
  const isOwner = account.user?.handle === profile.handle

  function toggleTheme() {
    const nextTheme = nextThemeMode(theme)
    applyTheme(nextTheme)
  }

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar contextLabel={profile.handle}>
          <div className="flex min-w-0 items-center gap-2">
            {account.user ? (
              <>
                <Button
                  aria-label="CLI sessions"
                  asChild
                  size="icon-sm"
                  title="CLI sessions"
                  type="button"
                  variant="secondary"
                >
                  <Link to="/account">
                    <KeyRound className="size-3.5" />
                  </Link>
                </Button>
                <UserButton />
              </>
            ) : (
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ _splat: '' }}
                  search={{ redirect_url: `/${profile.handle}` }}
                  to="/sign-in/$"
                >
                  Sign in
                </Link>
              </Button>
            )}
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
        </ApplicationTopbar>
      )}
    >
      <PageContent>
        <PageHeader description="Repositories" title={`@${profile.handle}`} />

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        <RepoList
          cliInstallCommands={state.cliInstallCommands}
          isOwner={isOwner}
          repositories={profile.repositories}
        />
      </PageContent>
    </AppShell>
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

  try {
    localStorage.setItem(THEME_STORAGE_KEY, theme)
  } catch {
    // ignore persistence failures (private mode, disabled storage)
  }

  window.dispatchEvent(new Event(THEME_CHANGE_EVENT))
}

function subscribeToTheme(onStoreChange: () => void) {
  if (typeof window === 'undefined') {
    return () => undefined
  }

  const observer = new MutationObserver(onStoreChange)
  observer.observe(document.documentElement, {
    attributeFilter: ['class'],
    attributes: true,
  })
  window.addEventListener('storage', onStoreChange)
  window.addEventListener(THEME_CHANGE_EVENT, onStoreChange)

  return () => {
    observer.disconnect()
    window.removeEventListener('storage', onStoreChange)
    window.removeEventListener(THEME_CHANGE_EVENT, onStoreChange)
  }
}

function readBrowserTheme(): ThemeMode {
  if (typeof document === 'undefined') {
    return 'dark'
  }

  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

function readServerTheme(): ThemeMode {
  return 'dark'
}

function nextThemeMode(theme: ThemeMode): ThemeMode {
  return theme === 'dark' ? 'light' : 'dark'
}
