import { cn } from "@/lib/utils";

/**
 * Flat toggle switch (P26 §二.1.3). `role="switch"` + `aria-checked` for
 * a11y/keyboard. Track uses the 4px radius (not a pill) to stay inside the
 * P25 §二 flat language; the knob is a 2px-radius square. No gloss / gradient.
 */
export interface SwitchProps {
  checked: boolean;
  onCheckedChange: (next: boolean) => void;
  disabled?: boolean;
  className?: string;
  "aria-label"?: string;
}

export function Switch({
  checked,
  onCheckedChange,
  disabled = false,
  className,
  "aria-label": ariaLabel,
}: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "inline-flex h-4 w-7 shrink-0 items-center rounded border border-border transition-colors",
        checked ? "border-accent bg-accent" : "bg-muted",
        disabled && "cursor-not-allowed opacity-50",
        className,
      )}
    >
      <span
        className={cn(
          "h-3 w-3 rounded-sm transition-transform duration-150",
          checked
            ? "translate-x-[14px] bg-accent-foreground"
            : "translate-x-0.5 bg-neutral-400",
        )}
      />
    </button>
  );
}
