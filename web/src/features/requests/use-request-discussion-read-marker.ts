import { useEffect, useEffectEvent, useRef, useState } from 'react'
import type { RequestDiscussion, RequestDiscussionView } from './request-discussion-types'

/**
 * Marks a thread read once its content has actually been on screen. The marker
 * sits after the thread body, so seeing it means everything above it was seen.
 */
export function useRequestDiscussionReadMarker({
  collapsed,
  contentFullyExposed,
  discussion,
  onMarkRead,
}: {
  collapsed: boolean
  contentFullyExposed: boolean
  discussion: RequestDiscussionView
  onMarkRead: (discussion: RequestDiscussion) => Promise<void>
}) {
  const markerRef = useRef<HTMLSpanElement>(null)
  const attemptedPositionRef = useRef<number | null>(null)
  const [markerVisible, setMarkerVisible] = useState(false)

  useEffect(() => {
    const marker = markerRef.current
    if (!marker || collapsed) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        const visible = Boolean(entry?.isIntersecting)
        if (!visible) attemptedPositionRef.current = null
        setMarkerVisible(visible)
      },
      { threshold: 1 },
    )
    observer.observe(marker)
    return () => observer.disconnect()
  }, [collapsed])

  const markRead = useEffectEvent(onMarkRead)

  useEffect(() => {
    if (
      !markerVisible ||
      collapsed ||
      discussion.unread_count === 0 ||
      discussion.pending ||
      !contentFullyExposed
    ) return
    if (attemptedPositionRef.current === discussion.last_activity_position) return
    attemptedPositionRef.current = discussion.last_activity_position
    void markRead(discussion)
  }, [markerVisible, collapsed, contentFullyExposed, discussion])

  return markerRef
}
