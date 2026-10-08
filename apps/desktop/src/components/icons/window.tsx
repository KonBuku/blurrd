import { createIcon } from "./icon";

/** Maximize button glyph: a rounded square frame. */
export const MaximizeIcon = createIcon(
  "MaximizeIcon",
  <rect x="5" y="5" width="14" height="14" rx="3" />,
);

/**
 * Restore button glyph: a smaller rounded square in front of the offset
 * outline of a second window.
 */
export const RestoreIcon = createIcon(
  "RestoreIcon",
  <>
    <rect x="4.5" y="8.5" width="11" height="11" rx="2.5" />
    <path d="M8.5 8.5V7a2.5 2.5 0 0 1 2.5-2.5h6A2.5 2.5 0 0 1 19.5 7v6a2.5 2.5 0 0 1-2.5 2.5h-1.5" />
  </>,
);
