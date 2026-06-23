import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { AlertDescription } from "@/components/ui/alert-description"
import { AlertTitle } from "@/components/ui/alert-title"
import { cn } from "@/lib/utils"

const alertVariants = cva(
  "group/alert relative grid w-full gap-0.5 rounded-lg border px-3 py-2.5 text-left text-sm has-[>svg]:grid-cols-[auto_1fr] has-[>svg]:gap-x-2 *:[svg]:row-span-2 *:[svg]:translate-y-0.5 *:[svg]:text-current *:[svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: "bg-background text-foreground",
        destructive:
          "border-destructive/30 bg-destructive/10 text-destructive *:data-[slot=alert-description]:text-destructive/90 *:[svg]:text-current",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

function Alert({
  className,
  live,
  variant,
  role,
  ...props
}: React.ComponentProps<"div"> &
  VariantProps<typeof alertVariants> & {
    live?: "assertive" | "polite" | "off"
  }) {
  const alertRole =
    role ??
    (variant === "destructive" || live === "assertive"
      ? "alert"
      : live === "polite"
        ? "status"
        : undefined)

  return (
    <div
      data-slot="alert"
      role={alertRole}
      className={cn(alertVariants({ variant }), className)}
      {...props}
    />
  )
}

export { Alert, AlertTitle, AlertDescription }
