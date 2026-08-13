import {
  createContext,
  type ReactNode,
  use,
  useLayoutEffect,
  useRef,
} from 'react'

type StoredPreview = {
  frame: HTMLIFrameElement
  identity: string
}

type PreviewStore = ReturnType<typeof createPreviewStore>

const RepositoryHtmlPreviewContext = createContext<PreviewStore | null>(null)

export function RepositoryHtmlPreviewProvider({
  children,
}: {
  children: ReactNode
}) {
  const storeRef = useRef<PreviewStore | null>(null)
  storeRef.current ??= createPreviewStore()

  return (
    <RepositoryHtmlPreviewContext.Provider value={storeRef.current}>
      {children}
      <div
        aria-hidden="true"
        className="pointer-events-none invisible fixed top-0 left-[-200vw]"
        ref={storeRef.current.setParkingHost}
      />
    </RepositoryHtmlPreviewContext.Provider>
  )
}

export function PersistentRepositoryHtmlPreview({
  className,
  identity,
  srcDoc,
  title,
}: {
  className: string
  identity: string
  srcDoc: string
  title: string
}) {
  const store = useRepositoryHtmlPreviewStore()
  const hostRef = useRef<HTMLDivElement>(null)
  const frameRef = useRef<HTMLIFrameElement>(null)
  const renderFrame = useRef(!store.has(identity)).current

  useLayoutEffect(() => {
    const host = hostRef.current
    if (!host) return
    return store.show(identity, host, frameRef.current)
  }, [identity, store])

  return (
    <div ref={hostRef}>
      {renderFrame && (
        <iframe
          className={className}
          ref={frameRef}
          referrerPolicy="no-referrer"
          sandbox=""
          srcDoc={srcDoc}
          title={title}
        />
      )}
    </div>
  )
}

function useRepositoryHtmlPreviewStore() {
  const store = use(RepositoryHtmlPreviewContext)
  if (!store) throw new Error('repository HTML preview store is unavailable')
  return store
}

function createPreviewStore() {
  let parkingHost: HTMLDivElement | null = null
  let preview: StoredPreview | null = null

  return {
    has(identity: string) {
      return preview?.identity === identity
    },
    setParkingHost(host: HTMLDivElement | null) {
      parkingHost = host
    },
    show(
      identity: string,
      host: HTMLDivElement,
      renderedFrame: HTMLIFrameElement | null,
    ) {
      if (preview?.identity !== identity) {
        preview?.frame.remove()
        if (!renderedFrame) {
          throw new Error('repository HTML preview frame is unavailable')
        }
        preview = { frame: renderedFrame, identity }
      } else if (renderedFrame && renderedFrame !== preview.frame) {
        renderedFrame.remove()
      }

      const activePreview = preview
      movePreview(host, activePreview.frame)

      return () => {
        if (preview !== activePreview || !parkingHost?.isConnected) return
        parkingHost.style.width = `${host.getBoundingClientRect().width}px`
        movePreview(parkingHost, activePreview.frame)
      }
    },
  }
}

function movePreview(parent: HTMLElement, frame: HTMLIFrameElement) {
  if (frame.parentElement === parent) return
  const canPreserveState =
    parent.isConnected &&
    frame.isConnected &&
    parent.getRootNode({ composed: true }) ===
      frame.getRootNode({ composed: true })
  if (
    canPreserveState &&
    'moveBefore' in parent &&
    typeof parent.moveBefore === 'function'
  ) {
    parent.moveBefore(frame, null)
  } else {
    parent.append(frame)
  }
}
