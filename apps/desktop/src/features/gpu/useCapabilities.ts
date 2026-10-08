import { useQuery } from "@tanstack/react-query";

import type { Capabilities } from "@/bindings/Capabilities";
import { getCapabilities } from "@/ipc/commands";

/** How often the capabilities are read again while the probe runs, in milliseconds. */
export const PROBE_POLL_MS = 500;

/**
 * The machine's capabilities. The first answer comes at once with
 * `probe_state: "Pending"`; the query polls until the background probe
 * reports `Ready`, then keeps the result for the session.
 */
export function useCapabilities() {
  return useQuery<Capabilities>({
    queryKey: ["capabilities"],
    queryFn: getCapabilities,
    refetchInterval: (query) =>
      query.state.data?.probe_state === "Ready" || query.state.error ? false : PROBE_POLL_MS,
  });
}
