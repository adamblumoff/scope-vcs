import { useEffect, useState } from 'react'

export type ThemeType = 'dark' | 'light'

function currentThemeType(): ThemeType {
  if (typeof document === 'undefined') {
    return 'dark'
  }
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

export function useThemeType(): ThemeType {
  const [theme, setTheme] = useState<ThemeType>(currentThemeType)

  useEffect(() => {
    const root = document.documentElement
    const update = () => setTheme(currentThemeType())
    update()
    const observer = new MutationObserver(update)
    observer.observe(root, { attributeFilter: ['class'], attributes: true })
    return () => observer.disconnect()
  }, [])

  return theme
}
