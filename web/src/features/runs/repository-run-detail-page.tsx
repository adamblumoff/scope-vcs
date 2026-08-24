import type {
  RepoRunDetail,
  RepoRunStepLogPage,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import { WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { useRepositoryRunDetailController } from './repository-run-detail-controller'
import { RunDetailHeader } from './run-detail-header'
import { RunDetailJobs } from './run-detail-jobs'

export function RepositoryRunDetailPage({
  cancelRun,
  initialDetail,
  loadDetail,
  loadLogs,
  params,
  retryRun,
}: {
  cancelRun: () => Promise<void>
  initialDetail: RepoRunDetail
  loadDetail: (signal?: AbortSignal) => Promise<RepoRunDetail>
  loadLogs: (
    input: RunStepLogsInput,
    signal?: AbortSignal,
  ) => Promise<RepoRunStepLogPage>
  params: RunActionInput
  retryRun: () => Promise<void>
}) {
  const {
    actionError,
    attemptOverrides,
    detail,
    metadataError,
    pendingAction,
    performAction,
    refreshDetail,
    refreshLogs,
    selectAttempt,
    selectedJobKey,
    selectedLogState,
    selection,
    showGraph,
    toggleGraph,
    toggleJob,
    toggleStep,
  } = useRepositoryRunDetailController({
    initialDetail,
    loadDetail,
    loadLogs,
    params,
  })

  return (
    <WorkbenchPane>
      <RunDetailHeader
        detail={detail}
        metadataError={metadataError}
        onCancel={() => void performAction('cancel', cancelRun)}
        onRefresh={() => void refreshDetail()}
        onRetry={() => void performAction('retry', retryRun)}
        params={params}
        pendingAction={pendingAction}
      />
      <main className="px-4 pb-14 sm:px-6 lg:px-8">
        {actionError ? (
          <div className="pt-5">
            <PageErrorAlert title="Run action failed">
              {actionError}
            </PageErrorAlert>
          </div>
        ) : null}
        <RunDetailJobs
          attemptOverrides={attemptOverrides}
          jobs={detail.jobs}
          onLogRetry={() => {
            if (selection) void refreshLogs(selection)
          }}
          onSelectAttempt={selectAttempt}
          onSelectJob={toggleJob}
          onSelectStep={toggleStep}
          onToggleGraph={toggleGraph}
          selectedJobKey={selectedJobKey}
          selectedLogState={selectedLogState}
          selection={selection}
          showGraph={showGraph}
        />
      </main>
    </WorkbenchPane>
  )
}

export function RunDetailPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected run detail error"
        title="Run unavailable"
      />
    </WorkbenchPane>
  )
}
