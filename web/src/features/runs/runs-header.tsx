import { WorkbenchHeader } from '@/components/workbench-header'

export function RunsHeader({
  runCount,
  workflowName,
}: {
  runCount?: number
  workflowName?: string
}) {
  return (
    <WorkbenchHeader
      actions={(
        <code className="max-w-full overflow-x-auto whitespace-nowrap text-xs text-muted-foreground">
          scope run &lt;workflow&gt;{' '}--runner &lt;name&gt;
        </code>
      )}
      count={runCount === undefined ? undefined : `${runCount} loaded`}
      description={(
        <>
          {workflowName ? `History for ${workflowName}. ` : null}
          Jobs from <code>.scope/runs</code> on your attached machines.
        </>
      )}
      eyebrow="Execute"
      title="Runs"
    />
  )
}
