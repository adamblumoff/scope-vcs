import { Button } from '@/components/ui/button'
import { toggleTheme, useThemeType } from '@/lib/use-theme-type'
import { Moon, Sun } from 'lucide-react'

export function ThemeToggle() {
  const theme = useThemeType()
  const label = `Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`

  return (
    <Button
      aria-label={label}
      onClick={toggleTheme}
      size="icon-sm"
      title={label}
      type="button"
      variant="ghost"
    >
      {theme === 'dark' ? <Sun /> : <Moon />}
    </Button>
  )
}
