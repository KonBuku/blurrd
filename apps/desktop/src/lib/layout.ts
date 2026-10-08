/**
 * Geometry of the window shell: the queue sidebar on the left, the preview
 * frame in the middle and the inspector frame on the right (design 8.0).
 *
 * Users drag the sidebar and inspector to a preferred width; the layout
 * clamps those preferences to the current window so the preview frame
 * always keeps a usable width. All values are CSS pixels.
 */

/** Queue sidebar width limits. */
export const SIDEBAR = { default: 280, min: 264, maxShare: 0.4 } as const;

/** Inspector width limits (frame and sheet). */
export const INSPECTOR = { default: 320, min: 300, max: 420 } as const;

/** Below this window width the inspector is an overlay sheet. */
export const INSPECTOR_SHEET_BREAKPOINT = 1180;

/** Narrowest preview frame the sidebar and inspector may leave. */
export const PREVIEW_MIN = 480;

/** Keyboard step of the resize handles. */
export const RESIZE_STEP = 16;

/**
 * Horizontal space taken by the 4 px gaps: sidebar | preview | inspector
 * | window edge.
 */
const GAPS = 12;

/** The user's layout preferences. */
export interface LayoutPreferences {
  readonly sidebarOpen: boolean;
  readonly sidebarWidth: number;
  readonly inspectorOpen: boolean;
  readonly inspectorWidth: number;
}

/** The layout for one window width. */
export interface ResolvedLayout {
  /** Whether the inspector is an overlay sheet instead of a frame. */
  readonly inspectorAsSheet: boolean;
  /** Sidebar width; 0 when it is hidden. */
  readonly sidebarWidth: number;
  /** Largest sidebar width the window allows right now. */
  readonly sidebarMax: number;
  /** Inspector width (frame or sheet); 0 when the frame is hidden. */
  readonly inspectorWidth: number;
  /** Largest inspector width the window allows right now. */
  readonly inspectorMax: number;
}

/** Clamps `value` to `min..max`; `min` wins when the range is empty. */
export function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Resolves the preferred widths for a window that is `windowWidth` wide.
 *
 * The inspector keeps its preferred width first (it holds the controls);
 * the sidebar then takes at most 40 % of the window and leaves the
 * preview at least {@link PREVIEW_MIN}. Minimum widths always win, which
 * is safe because the window itself is at least 1024 px wide.
 */
export function resolveLayout(windowWidth: number, prefs: LayoutPreferences): ResolvedLayout {
  const inspectorAsSheet = windowWidth < INSPECTOR_SHEET_BREAKPOINT;
  const inspectorFrame = !inspectorAsSheet && prefs.inspectorOpen;
  const preferredInspector = clamp(prefs.inspectorWidth, INSPECTOR.min, INSPECTOR.max);
  const inspectorSpace = inspectorFrame ? preferredInspector : 0;

  const sidebarMax = Math.max(
    SIDEBAR.min,
    Math.min(
      Math.floor(windowWidth * SIDEBAR.maxShare),
      windowWidth - inspectorSpace - PREVIEW_MIN - GAPS,
    ),
  );
  const sidebarWidth = prefs.sidebarOpen ? clamp(prefs.sidebarWidth, SIDEBAR.min, sidebarMax) : 0;

  const inspectorMax = inspectorAsSheet
    ? INSPECTOR.max
    : clamp(windowWidth - sidebarWidth - PREVIEW_MIN - GAPS, INSPECTOR.min, INSPECTOR.max);
  let inspectorWidth = 0;
  if (inspectorAsSheet) inspectorWidth = preferredInspector;
  else if (inspectorFrame) inspectorWidth = clamp(preferredInspector, INSPECTOR.min, inspectorMax);

  return { inspectorAsSheet, sidebarWidth, sidebarMax, inspectorWidth, inspectorMax };
}
