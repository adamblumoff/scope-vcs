import { FileSearch } from 'lucide-react'

export function ReviewEmptyFiles({
  description,
  title,
}: {
  description: string
  title: string
}) {
  return (
    <div className="flex items-center gap-3.5 py-8">
      <div className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-brand-muted text-brand">
        <FileSearch className="size-5" />
      </div>
      <div className="min-w-0 text-sm">
        <div className="text-base font-semibold leading-6">{title}</div>
        <div className="mt-0.5 leading-5 text-muted-foreground">
          {description}
        </div>
      </div>
    </div>
  )
}
