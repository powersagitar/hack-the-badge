import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"
import { Slot } from "radix-ui"

/**
 * shadcn's Button (shadcn@4.23.0, new-york), restyled to the design's
 * `.ghost`: JetBrains Mono 600 12 px, uppercase, `.06em` tracking, 40 px
 * tall, a 6 px radius, the field border and background. `on` is the
 * design's `.ghost.on` (a toggled-on button). The design's `.pill`
 * belonged to the removed mode toggle, so it has no variant.
 */
const buttonVariants = cva(
  "inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 rounded-control border font-mono text-xs leading-none font-semibold tracking-[.06em] whitespace-nowrap uppercase transition-colors outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/35 disabled:cursor-not-allowed disabled:opacity-38 aria-invalid:border-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "border-input bg-field text-secondary-foreground hover:border-field-hover-border hover:text-white disabled:hover:border-input disabled:hover:text-secondary-foreground",
        on: "border-primary bg-accent text-accent-foreground",
      },
      size: {
        default: "min-h-10 px-3.5",
        icon: "size-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
