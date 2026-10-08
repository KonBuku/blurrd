import { QueryClientProvider } from "@tanstack/react-query";

import { Toaster } from "@/components/ui/toaster";
import { AppShell } from "@/features/shell/AppShell";
import { queryClient } from "@/lib/query";

/**
 * The root component of the desktop app: the window shell and the toast
 * stack, inside the query client for backend reads.
 */
export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AppShell />
      <Toaster />
    </QueryClientProvider>
  );
}
