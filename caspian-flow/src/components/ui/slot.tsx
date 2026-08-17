import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Minimal Slot — renders its single child element, merging className and
 * props onto it. Enough for `asChild` button/link patterns without pulling in
 * @radix-ui/react-slot.
 */
export const Slot = React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement> & { children?: React.ReactNode }>(
  ({ children, className, ...props }, ref) => {
    if (!React.isValidElement(children)) {
      if (React.Children.count(children) > 1) {
        React.Children.only(null);
      }
      return null;
    }
    const child = children as React.ReactElement<Record<string, unknown>>;
    const childProps = child.props;
    return React.cloneElement(child, {
      ...props,
      ...childProps,
      className: cn(className, childProps.className as string | undefined),
      ref,
    } as Record<string, unknown>);
  },
);
Slot.displayName = "Slot";
