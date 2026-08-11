import { WorkbenchBar } from '@/components/page-header'

export function RunsHeader({
  runCount,
  workflowName,
}: {
  runCount?: number
  workflowName?: string
}) {
  return (
    <WorkbenchBar
      actions={(
        <code className="max-w-full overflow-x-auto whitespace-nowrap text-xs text-muted-foreground">
          scope run &lt;workflow&gt; --runner &lt;name&gt;
        </code>
      )}
      summary={
        runCount === undefined
          ? undefined
          : `${runCount} ${runCount === 1 ? 'run' : 'runs'}${workflowName ? ` in ${workflowName}` : ''}`
      }
      title="Runs"
    />
  )
}
