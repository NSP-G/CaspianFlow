import { cva, type VariantProps } from "class-variance-authority";

/**
 * Button variants — flattened to CaspianFlow §二 tokens (4px radius, no shadow,
 * neutral + single cold-gray-blue accent). Kept in its own file so `Button`
 * remains the only export of button.tsx (react-refresh fast-refresh rule).
 */
export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap text-[13px] font-medium transition-colors outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 select-none",
  {
    variants: {
      variant: {
        default:
          "bg-accent text-accent-foreground hover:bg-accent/90 border border-transparent",
        // Explicit primary alias — P26 导入按钮按报告要求用 btn-primary。
        primary:
          "bg-accent text-accent-foreground hover:bg-accent/90 border border-transparent",
        outline:
          "border border-border bg-transparent text-foreground hover:bg-muted",
        ghost: "bg-transparent text-foreground hover:bg-muted border border-transparent",
        subtle: "bg-muted text-foreground hover:bg-neutral-200 dark:hover:bg-neutral-700 border border-transparent",
      },
      size: {
        default: "h-8 px-3 rounded",
        sm: "h-7 px-2 rounded-sm text-xs",
        lg: "h-9 px-4 rounded",
        icon: "h-8 w-8 rounded p-0",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export type ButtonVariantProps = VariantProps<typeof buttonVariants>;
