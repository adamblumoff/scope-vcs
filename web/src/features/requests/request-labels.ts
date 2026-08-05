import type {
  RequestEvent,
  RequestListItem,
  RequestSummary,
  RequestWorkflowEventKind,
  RequestWorkflowState,
} from '@/api/types'
import type { BadgeVariant } from '@/components/ui/badge'

const REQUEST_DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  month: 'short',
  timeZone: 'UTC',
  year: 'numeric',
})

export type BadgeTone = BadgeVariant

const REQUEST_STATES = {
  Draft: { label: 'Draft', tone: 'neutral' },
  Open: { label: 'Open', tone: 'info' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
} as const satisfies Record<
  RequestWorkflowState,
  { label: string; tone: BadgeTone }
>

const EVENT_LABELS = {
  Started: 'Started',
  Submitted: 'Submitted',
  RevisionPushed: 'Revision pushed',
  Merged: 'Merged',
  Closed: 'Closed',
  IdentityEdited: 'Request edited',
  DiscussionResolved: 'Discussion resolved',
  DiscussionReopened: 'Discussion reopened',
} as const satisfies Record<RequestWorkflowEventKind, string>

const MERGEABILITY = {
  Ready: { label: 'Clean merge available', tone: 'success' },
  Draft: { label: 'Draft', tone: 'neutral' },
  Closed: { label: 'Closed', tone: 'neutral' },
  Merged: { label: 'Merged', tone: 'success' },
  NotMaintainer: { label: 'Maintainer required', tone: 'neutral' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
} as const satisfies Record<
  RequestSummary['mergeability']['status'],
  { label: string; tone: BadgeTone }
>

type RequestLabelSource = RequestSummary | RequestListItem

export function requestStatusLabel(request: RequestLabelSource) {
  return REQUEST_STATES[request.state].label
}

export function requestStatusTone(request: RequestLabelSource): BadgeTone {
  return REQUEST_STATES[request.state].tone
}

export function requestAudienceLabel(request: RequestLabelSource) {
  return request.audience === 'Private' ? 'Private request' : 'Public request'
}

export function requestAuthorRoleLabel(request: RequestLabelSource) {
  switch (request.author_role) {
    case 'Owner':
      return 'Owner'
    case 'Member':
      return 'Member'
    case 'Public':
      return 'Public contributor'
  }
}

export function eventKindLabel(kind: RequestWorkflowEventKind) {
  return EVENT_LABELS[kind]
}

export function requestMergeabilityLabel(request: RequestLabelSource) {
  return MERGEABILITY[request.mergeability.status].label
}

export function requestMergeabilityTone(request: RequestLabelSource): BadgeTone {
  return MERGEABILITY[request.mergeability.status].tone
}

export function requestEventBody(event: RequestEvent) {
  const payload = event.payload as unknown as Record<
    string,
    Record<string, unknown>
  >
  const value = payload[event.kind]
  if (!value) return null
  switch (event.kind) {
    case 'Started':
      return 'Initial request identity recorded.'
    case 'Submitted':
      return oidText(value.head_oid)
    case 'RevisionPushed':
      return [
        `${oidText(value.old_head_oid)} → ${oidText(value.new_head_oid)}`,
        stringValue(value.note),
      ]
        .filter(Boolean)
        .join('\n')
    case 'Closed':
      return oidText(value.head_oid)
    case 'Merged':
      return `${oidText(value.head_oid)} → ${oidText(value.main_oid)}`
    case 'IdentityEdited':
      return 'The request title or description was updated.'
    case 'DiscussionResolved':
    case 'DiscussionReopened':
      return value.discussion_id
        ? `Discussion ${stringValue(value.discussion_id)}`
        : null
  }
}

export function shortOid(oid: string | null | undefined) {
  if (!oid) {
    return 'none'
  }
  return oid.length > 12 ? oid.slice(0, 12) : oid
}

export function formatUnixDate(unixSeconds: number | null) {
  if (unixSeconds === null) {
    return 'Not set'
  }
  return REQUEST_DATE_FORMATTER.format(new Date(unixSeconds * 1000))
}

function oidText(value: unknown) {
  return typeof value === 'string' ? shortOid(value) : null
}

function stringValue(value: unknown) {
  return typeof value === 'string' && value.trim() ? value : null
}
