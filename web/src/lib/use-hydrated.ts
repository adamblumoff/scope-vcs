import { useSyncExternalStore } from 'react'

function subscribe() {
  return () => {}
}

/** Whether the component is rendering in the browser after hydration. */
export function useHydrated() {
  return useSyncExternalStore(
    subscribe,
    () => true,
    () => false,
  )
}
