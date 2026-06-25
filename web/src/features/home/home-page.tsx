import type { HomeState } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useHomeFlash } from '@/lib/home-flash'
import { UserButton, useUser } from '@clerk/tanstack-react-start'
import { useRouter } from '@tanstack/react-router'
import { CheckCircle2, Moon, Sun } from 'lucide-react'
import { useEffect, useState } from 'react'
import { RepoList } from './repo-list'

type ThemeMode = 'dark' | 'light'

export function HomePage({ home }: { home: HomeState }) {
  const [theme, setTheme] = useState<ThemeMode>('dark')
  const flash = useHomeFlash()
  const router = useRouter()
  const clerkUser = useUser()
  const { account, repositories } = home
  const signedIn = clerkUser.isLoaded ? clerkUser.isSignedIn : home.signedIn

  useEffect(() => {
    if (!clerkUser.isLoaded || clerkUser.isSignedIn === home.signedIn) {
      return
    }

    void router.invalidate()
  }, [clerkUser.isLoaded, clerkUser.isSignedIn, home.signedIn, router])

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
            {signedIn && <UserButton />}
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
          cliInstallCommand={home.cliInstallCommand}
          repositories={repositories}
          signedIn={signedIn}
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
}

function nextThemeMode(theme: ThemeMode): ThemeMode {
  return theme === 'dark' ? 'light' : 'dark'
}
