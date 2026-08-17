import * as React from "react";
import { cn } from "@/lib/utils";

/** Flattened text input — 4px radius, 1px border, no shadow, no glow. */
export const Input = React.forwardRef<
  HTMLInputElement,
  React.InputHTMLAttributes<HTMLInputElement>
>(({ className, type = "text", ...props }, ref) => {
  return (
    <input
      ref={ref}
      type={type}
      className={cn(
        "flex h-8 w-full rounded border border-input bg-transparent px-2.5 py-1 text-[13px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
});
Input.displayName = "Input";
