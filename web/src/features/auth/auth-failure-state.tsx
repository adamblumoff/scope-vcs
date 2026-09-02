import { PageErrorAlert } from '@/components/page-error-alert'
import { Button } from '@/components/ui/button'

export function AuthFailureState({ title }: { title: string }) {
  return (
    <>
      <PageErrorAlert className="mt-0" title={title}>
        Scope couldn't reach the sign-in service. Try again.
      </PageErrorAlert>
      <Button className="mt-4" onClick={() => window.location.reload()}>
        Try again
      </Button>
    </>
  )
}
