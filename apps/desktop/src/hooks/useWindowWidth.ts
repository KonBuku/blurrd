import { useSyncExternalStore } from "react";

function subscribe(onChange: () => void): () => void {
  window.addEventListener("resize", onChange);
  return () => window.removeEventListener("resize", onChange);
}

function snapshot(): number {
  return window.innerWidth;
}

/** The current viewport width in CSS pixels, updated on every resize. */
export function useWindowWidth(): number {
  return useSyncExternalStore(subscribe, snapshot);
}
