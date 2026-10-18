import { useQuery } from "@tanstack/react-query";

import { busyRetryDelay, loadObjectUrl, retryWhenBusy } from "@/ipc/media";

/**
 * Loads an image of the `blurrd://` scheme (thumbnail, poster) into an
 * object URL, cached for the session. A 503 (the backend's request queue
 * was full) is retried shortly, so a burst of images does not leave blank
 * slots. Nothing is loaded while `url` is `null`.
 */
export function useSchemeImage(url: string | null) {
  return useQuery({
    queryKey: ["scheme-image", url],
    queryFn: ({ signal }) => loadObjectUrl(url ?? "", signal),
    enabled: url !== null,
    retry: retryWhenBusy,
    retryDelay: busyRetryDelay,
  });
}
