export function runJobPanelId(jobKey: string) {
  return `run-job-${jobKey.replace(/[^a-zA-Z0-9_-]/g, '-')}`
}
