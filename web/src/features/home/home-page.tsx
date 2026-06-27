import type { HomeState } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useHomeFlash } from '@/lib/home-flash'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link } from '@tanstack/react-router'
import { CheckCircle2, KeyRound, Moon, Sun } from 'lucide-react'
import { useEffect, useState } from 'react'
import { RepoList } from './repo-list'

type ThemeMode = 'dark' | 'light'

const THEME_STORAGE_KEY = 'scope-theme'

export function HomePage({ home }: { home: HomeState }) {
  const [theme, setTheme] = useState<ThemeMode>('dark')
  const flash = useHomeFlash()
  const { account, repositories } = home

  useEffect(() => {
    setTheme(readStoredTheme())
  }, [])

  function toggleTheme() {
    const nextTheme = nextThemeMode(theme)
    setTheme(nextTheme)
    applyTheme(nextTheme)
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        action={
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
        }
        homeLink={false}
        subtitle={account?.user?.handle ?? 'Repositories'}
      />

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
}

function readStoredTheme(): ThemeMode {
  if (typeof document === 'undefined') {
    return 'dark'
  }

  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

function nextThemeMode(theme: ThemeMode): ThemeMode {
  return theme === 'dark' ? 'light' : 'dark'
}
