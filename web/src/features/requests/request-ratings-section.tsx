import type { RequestParams, RequestRating, RequestRatings } from '@/api/types'
import type { RateRequestInput } from '@/api/requests'
import { Button } from '@/components/ui/button'
import { Star } from 'lucide-react'
import { type FormEvent, useReducer } from 'react'

type FormState = {
  error: string | null
  reason: string
  score: number
  submitting: boolean
}

type FormAction =
  | { type: 'score_changed'; score: number }
  | { type: 'reason_changed'; reason: string }
  | { type: 'submission_started' }
  | { type: 'submission_succeeded' }
  | { type: 'submission_failed'; error: string }

const initialFormState: FormState = {
  error: null,
  reason: '',
  score: 5,
  submitting: false,
}

function formReducer(state: FormState, action: FormAction): FormState {
  switch (action.type) {
    case 'score_changed': return { ...state, score: action.score }
    case 'reason_changed': return { ...state, reason: action.reason }
    case 'submission_started': return { ...state, error: null, submitting: true }
    case 'submission_succeeded': return { ...state, reason: '', submitting: false }
    case 'submission_failed': return { ...state, error: action.error, submitting: false }
  }
}

export function RequestRatingsSection({
  initial,
  onRate,
  params,
}: {
  initial: RequestRatings
  onRate: (input: RateRequestInput) => Promise<RequestRating>
  params: RequestParams
}) {
  const [{ error, reason, score, submitting }, dispatch] = useReducer(
    formReducer,
    initialFormState,
  )
  const { eligible_subject: eligibleSubject, ratings } = initial

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!eligibleSubject || submitting) return
    dispatch({ type: 'submission_started' })
    try {
      await onRate({ ...params, reason, score })
      dispatch({ type: 'submission_succeeded' })
    } catch (cause) {
      dispatch({
        type: 'submission_failed',
        error: cause instanceof Error ? cause.message : 'Could not submit rating.',
      })
    }
  }

  return (
    <section>
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground">
        <Star className="size-3.5" />
        <h2>Participant ratings</h2>
      </div>
      {ratings.length ? (
        <div className="mt-3 divide-y divide-border">
          {ratings.map((rating) => (
            <div className="py-3 text-xs leading-5" key={rating.id}>
              <div className="font-medium">
                @{rating.rater.handle} rated @{rating.subject.handle} {rating.score}/5
              </div>
              <div className="text-muted-foreground">
                @{rating.subject.handle}: {rating.subject.rating_count} ratings ·{' '}
                {rating.subject.rating_score_sum} points
              </div>
              <p className="mt-1 whitespace-pre-wrap text-muted-foreground">{rating.reason}</p>
            </div>
          ))}
        </div>
      ) : (
        <p className="mt-3 text-xs leading-5 text-muted-foreground">No participant ratings yet.</p>
      )}

      {eligibleSubject ? (
        <form className="mt-4 grid gap-3 border-t border-border pt-4" onSubmit={submit}>
          <label className="grid gap-1 text-xs font-medium">
            Rating for @{eligibleSubject.handle}
            <select
              className="h-9 rounded-md border border-input bg-background px-2 text-sm"
              disabled={submitting}
              onChange={(event) => dispatch({
                type: 'score_changed',
                score: Number(event.target.value),
              })}
              value={score}
            >
              {[5, 4, 3, 2, 1].map((value) => (
                <option key={value} value={value}>{value} / 5</option>
              ))}
            </select>
          </label>
          <label className="grid gap-1 text-xs font-medium">
            Reason
            <textarea
              className="min-h-24 resize-y rounded-md border border-input bg-background px-3 py-2 text-sm font-normal"
              disabled={submitting}
              maxLength={1024}
              onChange={(event) => dispatch({
                type: 'reason_changed',
                reason: event.target.value,
              })}
              required
              value={reason}
            />
          </label>
          {error ? <p className="text-xs text-destructive" role="alert">{error}</p> : null}
          <Button disabled={submitting || !reason.trim()} size="sm" type="submit">
            {submitting ? 'Submitting…' : 'Submit rating'}
          </Button>
        </form>
      ) : null}
    </section>
  )
}
