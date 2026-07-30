const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  dateStyle: 'medium',
  timeStyle: 'short',
  timeZone: 'UTC',
})

export function formatRunUnixTime(value: number) {
  return DATE_FORMATTER.format(new Date(value * 1_000))
}
