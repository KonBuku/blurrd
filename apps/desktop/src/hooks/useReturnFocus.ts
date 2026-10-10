import { useRef } from "react";

/** Auto-focus handlers of a Radix dialog content. */
export interface ReturnFocusHandlers {
  readonly onOpenAutoFocus: () => void;
  readonly onCloseAutoFocus: (event: Event) => void;
}

/**
 * Focus handling for a dialog opened without a trigger element (from the
 * command palette or a shortcut): Radix would move the focus to the
 * missing trigger, so the element focused before the dialog opened gets
 * it back instead.
 */
export function useReturnFocus(): ReturnFocusHandlers {
  const previous = useRef<HTMLElement | null>(null);
  return {
    onOpenAutoFocus: () => {
      const active = document.activeElement;
      previous.current = active instanceof HTMLElement ? active : null;
    },
    onCloseAutoFocus: (event) => {
      event.preventDefault();
      if (previous.current?.isConnected) previous.current.focus();
      previous.current = null;
    },
  };
}
