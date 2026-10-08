import { Toaster } from "@/components/ui/toaster";
import { AppShell } from "@/features/shell/AppShell";

/** The root component of the desktop app: the window shell and the toast stack. */
export function App() {
  return (
    <>
      <AppShell />
      <Toaster />
    </>
  );
}
