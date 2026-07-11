import type {
  RequestEvent,
  RequestSummary,
  RequestWorkflowDisposition,
  RequestWorkflowResolutionDisposition,
  RequestWorkflowEventKind,
  RequestWorkflowState,
} from '@/api/types'
import type { BadgeVariant } from '@/components/ui/badge'

export type BadgeTone = BadgeVariant

export type ResolutionOption = {
  description: string
  disposition: RequestWorkflowResolutionDisposition
  label: string
}

export type SettlementPreview = {
  burnedCredits: number
  refundedCredits: number
  rewardCredits: number
  stakeCredits: number
}

const RESOLUTION_OPTIONS = [
  {
    description: 'Good work that helped, but the repo chose another path.',
    disposition: 'UsefulNotMerged',
    label: 'Useful, not merged',
  },
  {
    description: 'The contributor could not reasonably know it was blocked.',
    disposition: 'HiddenContext',
    label: 'Blocked by hidden context',
  },
  {
    description: 'Reasonable work, but not aligned with current project direction.',
    disposition: 'NotAligned',
    label: 'Reasonable, not aligned',
  },
  {
    description: 'Missed context that was visible before submitting.',
    disposition: 'Duplicate',
    label: 'Duplicate or obvious miss',
  },
  {
    description: 'The contributor disappeared after maintainer follow-up.',
    disposition: 'Abandoned',
    label: 'Abandoned',
  },
  {
    description: 'Low-signal, misleading, spammy, or not good-faith.',
    disposition: 'LowQuality',
    label: 'Low quality',
  },
] as const satisfies ResolutionOption[]

const REQUEST_STATES = {
  Working: { label: 'Working', tone: 'neutral' },
  Submitted: { label: 'Submitted', tone: 'info' },
  NeedsResponse: { label: 'Needs response', tone: 'warning' },
  Resolved: { label: 'Resolved', tone: 'success' },
  Withdrawn: { label: 'Withdrawn', tone: 'neutral' },
} as const satisfies Record<RequestWorkflowState, { label: string; tone: BadgeTone }>

const DISPOSITIONS = {
  Accepted: { label: 'Accepted', tone: 'success' },
  UsefulNotMerged: { label: 'Useful, not merged', tone: 'success' },
  HiddenContext: { label: 'Blocked by hidden context', tone: 'success' },
  NotAligned: { label: 'Reasonable, not aligned', tone: 'neutral' },
  Duplicate: { label: 'Duplicate or obvious miss', tone: 'warning' },
  Abandoned: { label: 'Abandoned', tone: 'danger' },
  LowQuality: { label: 'Low quality', tone: 'danger' },
} as const satisfies Record<RequestWorkflowDisposition, { label: string; tone: BadgeTone }>

const EVENT_LABELS = {
  Started: 'Started', Submitted: 'Submitted', RevisionPushed: 'Revision pushed',
  Commented: 'Commented', NeedsResponse: 'Needs response',
  ContributorResponded: 'Contributor responded', Merged: 'Merged',
  Resolved: 'Resolved', Settled: 'Settled', Withdrawn: 'Withdrawn',
} as const satisfies Record<RequestWorkflowEventKind, string>

const MERGEABILITY = {
  Ready: { label: 'Clean merge available', tone: 'success' },
  Closed: { label: 'Closed', tone: 'neutral' },
  NotReady: { label: 'Not ready', tone: 'warning' },
  NotMaintainer: { label: 'Maintainer required', tone: 'neutral' },
  MissingRequestBranch: { label: 'Branch missing', tone: 'warning' },
} as const satisfies Record<RequestSummary['mergeability']['status'], { label: string; tone: BadgeTone }>

export function requestStatusLabel(request: RequestSummary) {
  return request.state === 'Resolved' && request.disposition
    ? dispositionLabel(request.disposition)
    : REQUEST_STATES[request.state].label
}

export function requestStatusTone(request: RequestSummary): BadgeTone {
  return request.state === 'Resolved'
    ? dispositionTone(request.disposition)
    : REQUEST_STATES[request.state].tone
}

export function dispositionLabel(disposition: RequestWorkflowDisposition) {
  return DISPOSITIONS[disposition].label
}

export function dispositionTone(
  disposition: RequestWorkflowDisposition | null,
): BadgeTone {
  return disposition ? DISPOSITIONS[disposition].tone : 'outline'
}

export function requestAudienceLabel(request: RequestSummary) {
  return request.audience === 'Private' ? 'Private request' : 'Public request'
}

export function requestAuthorRoleLabel(request: RequestSummary) {
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

export function requestMergeabilityLabel(request: RequestSummary) {
  return MERGEABILITY[request.mergeability.status].label
}

export function requestMergeabilityTone(request: RequestSummary): BadgeTone {
  return MERGEABILITY[request.mergeability.status].tone
}

export function resolutionOptionsFor(
  request: RequestSummary,
): ResolutionOption[] {
  const allowed = new Set(request.resolution_options.map(({ disposition }) => disposition))
  return RESOLUTION_OPTIONS.filter(({ disposition }) => allowed.has(disposition))
}

export function settlementPreviewFor(
  request: RequestSummary,
  disposition: RequestWorkflowDisposition,
): SettlementPreview {
  const preview = disposition === 'Accepted'
    ? request.merge_settlement_preview
    : request.resolution_options.find((option) => option.disposition === disposition)?.settlement
  if (!preview) throw new Error(`resolution ${disposition} is not allowed`)
  return {
    burnedCredits: preview.burned_credits,
    refundedCredits: preview.refunded_credits,
    rewardCredits: preview.reward_credits,
    stakeCredits: preview.stake_credits,
  }
}

export function settlementPreviewText(preview: SettlementPreview) {
  return [
    `${preview.refundedCredits} refunded`,
    `${preview.rewardCredits} reward`,
    `${preview.burnedCredits} burned`,
  ].join(' / ')
}

export function requestEventBody(event: RequestEvent) {
  if (event.body) {
    return event.body
  }
  if (event.old_head_oid && event.new_head_oid) {
    return `${shortOid(event.old_head_oid)} -> ${shortOid(event.new_head_oid)}`
  }
  if (event.new_head_oid) {
    return shortOid(event.new_head_oid)
  }
  return null
}

export function shortOid(oid: string | null | undefined) {
  if (!oid) {
    return 'none'
  }
  return oid.length > 12 ? oid.slice(0, 12) : oid
}

export function fullOid(oid: string | null | undefined) {
  return oid ?? 'none'
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

export function normalizedBody(body: string) {
  const trimmed = body.trim()
  return trimmed ? trimmed : null
}
