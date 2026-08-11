import { useSyncExternalStore } from 'react'

export type ThemeType = 'dark' | 'light'

const THEME_STORAGE_KEY = 'scope-theme'
const THEME_CHANGE_EVENT = 'scope-theme-change'

/** Single source of truth for the active theme. The inline boot script in
 * `__root.tsx` sets the class before hydration; this reads it back. */
export function useThemeType(): ThemeType {
  return useSyncExternalStore(subscribe, readBrowserTheme, readServerTheme)
}

export function toggleTheme() {
  applyTheme(readBrowserTheme() === 'dark' ? 'light' : 'dark')
}

function applyTheme(theme: ThemeType) {
  if (typeof document === 'undefined') return

  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme

  try {
    localStorage.setItem(THEME_STORAGE_KEY, theme)
  } catch {
    // ignore persistence failures (private mode, disabled storage)
  }

  window.dispatchEvent(new Event(THEME_CHANGE_EVENT))
}

function subscribe(onStoreChange: () => void) {
  if (typeof window === 'undefined') return () => undefined

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

function readBrowserTheme(): ThemeType {
  if (typeof document === 'undefined') return 'dark'
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light'
}

function readServerTheme(): ThemeType {
  return 'dark'
}
