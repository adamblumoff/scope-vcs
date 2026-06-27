import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const badgeVariants = cva(
  "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-md border px-1.5 py-0.5 text-xs font-medium whitespace-nowrap focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-3!",
  {
    variants: {
      variant: {
        outline:
          "border-border bg-background text-foreground [a]:hover:bg-muted [a]:hover:text-muted-foreground",
        neutral: "border-transparent bg-muted text-muted-foreground",
        success:
          "border-green-400 bg-green-100 text-green-900",
        warning:
          "border-amber-400 bg-amber-100 text-amber-900",
        danger:
          "border-red-400 bg-red-100 text-red-900 dark:border-red-500/60 dark:bg-red-500/15 dark:text-red-300",
        info:
          "border-blue-400 bg-blue-100 text-blue-900",
      },
    },
    defaultVariants: {
      variant: "outline",
    },
  }
)

function Badge({
  className,
  variant = "outline",
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return (
    <span
      data-slot="badge"
      data-variant={variant}
      className={cn(badgeVariants({ variant }), className)}
      {...props}
    />
  )
}

export { Badge }
