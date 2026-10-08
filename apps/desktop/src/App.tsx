import { TitleBar } from "@/features/shell/TitleBar";

/** The root component of the desktop app. */
export function App() {
  return (
    <div className="flex h-full flex-col bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <TitleBar />
      <main className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2">
        <h1 className="font-semibold">blurrd</h1>
        <p className="text-neutral-500">Blur player names in gameplay recordings.</p>
      </main>
    </div>
  );
}
