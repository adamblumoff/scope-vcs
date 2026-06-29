import { loadRepoLiveStateForRequest, parseRepoParams } from '@/api/repos'
import { loadProjectionPreviewsForRequest } from '@/api/projection-preview'
import {
  loadReviewFileDiffForRequest,
  loadReviewForRequest,
  parseSetVisibilityInput,
  parseReviewFileDiffInput,
  postStagedUpdateAction,
  publishRepoForRequest,
  setVisibilityForRequest,
} from '@/api/review'
import type { RepoParams, RepoReview, ReviewFile, Visibility } from '@/api/types'
import { createServerFn } from '@tanstack/react-start'

export const loadReview = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadReviewForRequest(data))

export const loadReviewRepoLiveState = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoLiveStateForRequest(data))

export const loadReviewProjectionPreviews = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadProjectionPreviewsForRequest(data, 'review'))

export const loadReviewFileDiff = createServerFn({ method: 'GET' })
  .validator(parseReviewFileDiffInput)
  .handler(({ data }) => loadReviewFileDiffForRequest(data))

export const setVisibility = createServerFn({ method: 'POST' })
  .validator(parseSetVisibilityInput)
  .handler(({ data }) => setVisibilityForRequest(data))

export const publishRepo = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => publishRepoForRequest(data))

export const applyStagedUpdate = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => postStagedUpdateAction(data, 'apply'))

export const rejectStagedUpdate = createServerFn({ method: 'POST' })
  .validator(parseRepoParams)
  .handler(({ data }) => postStagedUpdateAction(data, 'reject'))

export function setReviewVisibility(
  params: RepoParams,
  review: RepoReview,
  files: ReviewFile[],
  visibility: Visibility,
) {
  return setVisibility({
    data: {
      ...params,
      kind: review.kind,
      paths: files.map((file) => file.path),
      visibility,
    },
  })
}
