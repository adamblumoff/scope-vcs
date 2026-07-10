import type { HomeState } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { AppShell } from '@/components/app-shell'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
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

export function HomePage({ home }: { home: HomeState }) {
  const theme = useSyncExternalStore(
    subscribeToTheme,
    readBrowserTheme,
    readServerTheme,
  )
  const flash = useHomeFlash()
  const { account, repositories } = home

  function toggleTheme() {
    const nextTheme = nextThemeMode(theme)
    applyTheme(nextTheme)
  }

  return (
    <AppShell
      header={() => (
        <AppHeader
          action={() => (
          <div className="flex min-w-0 items-center gap-2">
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
            <ThemeToggle theme={theme} toggleTheme={toggleTheme} />
          </div>
          )}
          homeLink={false}
          subtitle={account?.user?.handle ?? 'Repositories'}
        />
      )}
    >
      <PageContent>
        <PageHeader
          description={account?.identity?.email ?? account?.identity?.user_id}
          title="Repositories"
        />

        {home.error && (
          <PageErrorAlert title="Repositories unavailable">
            {home.error}
          </PageErrorAlert>
        )}

        {flash && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Success</AlertTitle>
            <AlertDescription>{flash}</AlertDescription>
          </Alert>
        )}

        <RepoList
          cliInstallCommands={home.cliInstallCommands}
          repositories={repositories}
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
