import { useQuery } from "@tanstack/react-query";

import { loadLicenses, type LicenseData } from "./licenses";

/** The licence data, loaded once on first use and kept for the session. */
export function useLicenses() {
  return useQuery<LicenseData>({
    queryKey: ["licenses"],
    queryFn: loadLicenses,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
  });
}
