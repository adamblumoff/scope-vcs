import { FileSearch } from 'lucide-react'

export function ReviewEmptyFiles({
  description,
  title,
}: {
  description: string
  title: string
}) {
  return (
    <div className="flex items-center gap-3 py-8">
      <div className="flex size-10 shrink-0 items-center justify-center rounded-md border border-border">
        <FileSearch className="size-5 text-muted-foreground" />
      </div>
      <div className="min-w-0 text-sm">
        <div className="font-medium leading-5">{title}</div>
        <div className="mt-1 leading-5 text-muted-foreground">
          {description}
        </div>
      </div>
    </div>
  )
}
