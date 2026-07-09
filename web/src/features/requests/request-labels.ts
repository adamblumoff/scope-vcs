import type {
  RequestEvent,
  RequestSummary,
  RequestWorkflowDisposition,
  RequestWorkflowEventKind,
  RequestWorkflowState,
} from '@/api/types'

export type BadgeTone =
  | 'danger'
  | 'info'
  | 'neutral'
  | 'outline'
  | 'success'
  | 'warning'

export type ResolutionOption = {
  description: string
  disposition: Exclude<RequestWorkflowDisposition, 'Accepted'>
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

export function requestStateLabel(state: RequestWorkflowState) {
  switch (state) {
    case 'Working':
      return 'Working'
    case 'Submitted':
      return 'Submitted'
    case 'NeedsResponse':
      return 'Needs response'
    case 'Resolved':
      return 'Resolved'
    case 'Withdrawn':
      return 'Withdrawn'
  }
}

export function requestStateTone(state: RequestWorkflowState): BadgeTone {
  switch (state) {
    case 'Submitted':
      return 'info'
    case 'NeedsResponse':
      return 'warning'
    case 'Resolved':
      return 'success'
    case 'Working':
    case 'Withdrawn':
      return 'neutral'
  }
}

export function dispositionLabel(disposition: RequestWorkflowDisposition) {
  switch (disposition) {
    case 'Accepted':
      return 'Accepted'
    case 'UsefulNotMerged':
      return 'Useful, not merged'
    case 'HiddenContext':
      return 'Blocked by hidden context'
    case 'NotAligned':
      return 'Reasonable, not aligned'
    case 'Duplicate':
      return 'Duplicate or obvious miss'
    case 'Abandoned':
      return 'Abandoned'
    case 'LowQuality':
      return 'Low quality'
  }
}

export function dispositionTone(
  disposition: RequestWorkflowDisposition | null,
): BadgeTone {
  switch (disposition) {
    case 'Accepted':
    case 'UsefulNotMerged':
    case 'HiddenContext':
      return 'success'
    case 'NotAligned':
      return 'neutral'
    case 'Duplicate':
      return 'warning'
    case 'Abandoned':
    case 'LowQuality':
      return 'danger'
    case null:
      return 'outline'
  }
}

export function requestBaseAudienceLabel(request: RequestSummary) {
  return request.base_audience === 'Private' ? 'Private base' : 'Public base'
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
  switch (kind) {
    case 'Started':
      return 'Started'
    case 'Submitted':
      return 'Submitted'
    case 'RevisionPushed':
      return 'Revision pushed'
    case 'Commented':
      return 'Commented'
    case 'NeedsResponse':
      return 'Needs response'
    case 'ContributorResponded':
      return 'Contributor responded'
    case 'Merged':
      return 'Merged'
    case 'Resolved':
      return 'Resolved'
    case 'Settled':
      return 'Settled'
    case 'Withdrawn':
      return 'Withdrawn'
  }
}

export function requestMergeabilityLabel(request: RequestSummary) {
  switch (request.mergeability.status) {
    case 'Ready':
      return 'Clean merge available'
    case 'Closed':
      return 'Closed'
    case 'NotReady':
      return 'Not ready'
    case 'NotMaintainer':
      return 'Maintainer required'
    case 'MissingRequestBranch':
      return 'Branch missing'
  }
}

export function requestMergeabilityTone(request: RequestSummary): BadgeTone {
  switch (request.mergeability.status) {
    case 'Ready':
      return 'success'
    case 'NotReady':
    case 'MissingRequestBranch':
      return 'warning'
    case 'Closed':
    case 'NotMaintainer':
      return 'neutral'
  }
}

export function resolutionOptionsFor(
  request: RequestSummary,
): ResolutionOption[] {
  return RESOLUTION_OPTIONS.filter(
    (option) =>
      option.disposition !== 'Abandoned' || request.state === 'NeedsResponse',
  )
}

export function settlementPreviewFor(
  stakeCredits: number,
  disposition: RequestWorkflowDisposition,
): SettlementPreview {
  const refundedCredits = refundedCreditsFor(stakeCredits, disposition)
  const rewardCredits = rewardCreditsFor(stakeCredits, disposition)

  return {
    burnedCredits: Math.max(0, stakeCredits - refundedCredits),
    refundedCredits,
    rewardCredits,
    stakeCredits,
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

function refundedCreditsFor(
  stakeCredits: number,
  disposition: RequestWorkflowDisposition,
) {
  switch (disposition) {
    case 'Accepted':
    case 'UsefulNotMerged':
    case 'HiddenContext':
    case 'NotAligned':
      return stakeCredits
    case 'Duplicate':
      return Math.floor(stakeCredits / 2)
    case 'Abandoned':
    case 'LowQuality':
      return 0
  }
}

function rewardCreditsFor(
  stakeCredits: number,
  disposition: RequestWorkflowDisposition,
) {
  switch (disposition) {
    case 'Accepted':
      return Math.floor(stakeCredits / 2)
    case 'UsefulNotMerged':
      return Math.floor(stakeCredits / 5)
    case 'HiddenContext':
    case 'NotAligned':
    case 'Duplicate':
    case 'Abandoned':
    case 'LowQuality':
      return 0
  }
}
