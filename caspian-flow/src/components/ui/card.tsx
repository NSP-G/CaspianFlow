import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Flat surface container (P26). A single 1px border + 4px radius, no shadow, no
 * lift — consistent with P25 §二 "card 用空白 + 单条细线分组，禁止卡中卡".
 * Compose inner structure with plain divs; no CardHeader/Content split needed.
 */
export function Card({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded border border-border bg-card text-card-foreground",
        className,
      )}
      {...props}
    />
  );
}
