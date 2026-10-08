import { QueryClient } from "@tanstack/react-query";

/**
 * The TanStack Query client for idempotent backend reads (plan 5.9:
 * capabilities, media facts, tracks). Desktop data does not go stale on
 * window focus, and a failed read is shown instead of retried silently.
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, refetchOnWindowFocus: false, staleTime: Number.POSITIVE_INFINITY },
  },
});
