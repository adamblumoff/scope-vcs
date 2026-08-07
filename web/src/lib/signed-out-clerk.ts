import type { ClerkProp } from '@clerk/tanstack-react-start'

const resources = {
  client: null,
  organization: null,
  session: null,
  user: null,
}

const noop = () => {}
let loaded = false

type StatusListener = (status: string) => void
const statusListeners = new Set<StatusListener>()

export const signedOutClerk = {
  __internal_lastEmittedResources: resources,
  __internal_updateProps: async () => {},
  addListener: () => noop,
  client: null,
  isSignedIn: false,
  load: async () => {
    // Clerk starts loading while React is hydrating. Keep the first client
    // render aligned with the server before publishing the ready state.
    await new Promise<void>((resolve) => setTimeout(resolve, 1_000))
    loaded = true
    statusListeners.forEach((listener) => listener('ready'))
  },
  get loaded() {
    return loaded
  },
  mountUserButton: noop,
  off: (event: string, listener: StatusListener) => {
    if (event === 'status') {
      statusListeners.delete(listener)
    }
  },
  on: (
    event: string,
    listener: StatusListener,
  ) => {
    if (event === 'status') {
      statusListeners.add(listener)
      listener(loaded ? 'ready' : 'loading')
    }
    return noop
  },
  organization: null,
  session: null,
  signOut: async () => {},
  get status() {
    return loaded ? 'ready' : 'loading'
  },
  telemetry: { record: noop },
  unmountUserButton: noop,
  user: null,
} as unknown as ClerkProp
