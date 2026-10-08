import brandIcon from "@brand/blurrd-icon-small.svg";
import { WindowControls } from "./WindowControls";

/**
 * The custom title bar (decision D6), 48 px tall on the window shell
 * colour. The bar and its empty areas are `data-tauri-drag-region`:
 * dragging moves the window with native drag-to-edge snapping, and a
 * double-click maximizes or restores it. Buttons inside the bar are not
 * drag regions.
 */
export function TitleBar() {
  return (
    <header
      data-tauri-drag-region
      className="flex h-12 shrink-0 items-center gap-2 pr-2 pl-3 select-none"
    >
      <img src={brandIcon} alt="" className="pointer-events-none size-5" />
      <span data-tauri-drag-region className="text-ui-base font-medium">
        blurrd
      </span>
      <div data-tauri-drag-region className="h-full min-w-0 flex-1" />
      <WindowControls />
    </header>
  );
}
