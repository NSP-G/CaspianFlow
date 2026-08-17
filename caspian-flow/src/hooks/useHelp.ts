import { useEffect, useState } from "react";

/**
 * F1 help-panel state + global shortcut (mirrors useCommandPalette for Cmd/Ctrl+K).
 * The panel renders on top of whatever page is active; F1 toggles it.
 */
export function useHelp() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "F1") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return { open, setOpen };
}
