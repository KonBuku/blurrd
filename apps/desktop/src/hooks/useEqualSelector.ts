import { useRef } from "react";

/**
 * Wraps a store selector so that it keeps answering its previous result
 * while `equal` says the new one is the same (like zustand's `useShallow`,
 * with a custom comparison). Components then render again only for the
 * changes they show, not for every update of the selected object, for
 * example a queue item whose job progress changes 20 times a second.
 */
export function useEqualSelector<S, U>(
  selector: (state: S) => U,
  equal: (previous: U, next: U) => boolean,
): (state: S) => U {
  const previous = useRef<{ readonly value: U } | null>(null);
  return (state) => {
    const next = selector(state);
    if (previous.current !== null && equal(previous.current.value, next)) {
      return previous.current.value;
    }
    previous.current = { value: next };
    return next;
  };
}
