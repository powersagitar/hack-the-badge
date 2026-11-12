import * as React from "react"
import { cn } from "@/lib/utils"

/**
 * shadcn's Input (shadcn@4.23.0, new-york), restyled to the design's
 * `.field`: JetBrains Mono 500 12 px, `.04em` tracking, 40 px tall, the
 * field border and background, and the accent focus ring.
 */

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "box-border min-h-10 w-full min-w-0 rounded-control border border-input bg-field px-3 font-mono text-xs leading-none font-medium tracking-[.04em] text-foreground transition-[color,box-shadow] outline-none selection:bg-primary selection:text-primary-foreground placeholder:text-placeholder disabled:cursor-not-allowed disabled:opacity-38",
        "focus:border-ring focus:ring-2 focus:ring-ring/35",
        "aria-invalid:border-destructive",
        className
      )}
      {...props}
    />
  )
}

export { Input }
