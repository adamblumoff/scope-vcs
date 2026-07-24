import type {
  RequestEvent,
  RequestListItem,
  RequestSummary,
  RequestWorkflowAssessmentOutcome,
  RequestWorkflowEventKind,
  RequestWorkflowState,
} from '@/api/types'
import type { BadgeVariant } from '@/components/ui/badge'

export type BadgeTone = BadgeVariant

const REQUEST_STATES = {
  Working: { label: 'Working', tone: 'neutral' },
  ReadyForReview: { label: 'Ready for review', tone: 'info' },
  Completed: { label: 'Completed', tone: 'success' },
} as const satisfies Record<
  RequestWorkflowState,
  { label: string; tone: BadgeTone }
>

const ASSESSMENTS = {
  Accepted: { label: 'Accepted', tone: 'success' },
  Neutral: { label: 'Neutral', tone: 'neutral' },
  Rejected: { label: 'Rejected', tone: 'danger' },
} as const satisfies Record<
  RequestWorkflowAssessmentOutcome,
  { label: string; tone: BadgeTone }
>

const EVENT_LABELS = {
  Started: 'Started',
  ReadyForReview: 'Ready for review',
  ReturnedToWorking: 'Returned to working',
  RevisionPushed: 'Revision pushed',
  Held: 'Held',
  HoldReleased: 'Hold released',
  Assessed: 'Assessed',
  Merged: 'Merged',
  Closed: 'Closed',
  Settled: 'Settled',
  IdentityEdited: 'Request edited',
  DiscussionResolved: 'Discussion resolved',
  DiscussionReopened: 'Discussion reopened',
} as const satisfies Record<RequestWorkflowEventKind, string>

const MERGEABILITY = {
  Ready: { label: 'Clean merge available', tone: 'success' },
  Completed: { label: 'Completed', tone: 'neutral' },
  Working: { label: 'Working', tone: 'neutral' },
  NotMaintainer: { label: 'Maintainer required', tone: 'neutral' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
} as const satisfies Record<
  RequestSummary['mergeability']['status'],
  { label: string; tone: BadgeTone }
>

type RequestLabelSource = RequestSummary | RequestListItem

export function requestStatusLabel(request: RequestLabelSource) {
  return request.state === 'Completed' && request.assessment_outcome
    ? ASSESSMENTS[request.assessment_outcome].label
    : REQUEST_STATES[request.state].label
}

export function requestStatusTone(request: RequestLabelSource): BadgeTone {
  return request.state === 'Completed' && request.assessment_outcome
    ? ASSESSMENTS[request.assessment_outcome].tone
    : REQUEST_STATES[request.state].tone
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
      return stringValue(value.title)
    case 'ReadyForReview':
      return [
        oidText(value.head_oid),
        creditText(value.stake_credits, 'staked'),
      ]
        .filter(Boolean)
        .join(' · ')
    case 'ReturnedToWorking':
      return [
        stringValue(value.reason),
        creditText(value.stake_credits, 'refunded'),
      ]
        .filter(Boolean)
        .join(' · ')
    case 'RevisionPushed':
      return [
        `${oidText(value.old_head_oid)} → ${oidText(value.new_head_oid)}`,
        stringValue(value.note),
      ]
        .filter(Boolean)
        .join('\n')
    case 'Held':
    case 'HoldReleased':
    case 'Closed':
      return oidText(value.head_oid)
    case 'Assessed':
      return [
        stringValue(value.outcome),
        stringValue(value.body_markdown),
      ]
        .filter(Boolean)
        .join(' · ')
    case 'Merged':
      return `${oidText(value.head_oid)} → ${oidText(value.main_oid)}`
    case 'Settled': {
      const settlement = value.settlement as Record<string, unknown> | undefined
      return settlement
        ? [
            `${numberValue(settlement.refunded_credits)} refunded`,
            `${numberValue(settlement.reward_credits)} reward`,
            `${numberValue(settlement.burned_credits)} burned`,
          ].join(' / ')
        : null
    }
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
  return new Intl.DateTimeFormat(undefined, {
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    month: 'short',
    year: 'numeric',
  }).format(new Date(unixSeconds * 1000))
}

function oidText(value: unknown) {
  return typeof value === 'string' ? shortOid(value) : null
}

function stringValue(value: unknown) {
  return typeof value === 'string' && value.trim() ? value : null
}

function numberValue(value: unknown) {
  return typeof value === 'number' ? value : 0
}

function creditText(value: unknown, suffix: string) {
  return typeof value === 'number' ? `${value} ${suffix}` : null
}
