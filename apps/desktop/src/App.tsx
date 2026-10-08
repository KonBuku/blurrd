/** The root component of the desktop app. */
export function App() {
  return (
    <main className="flex h-full flex-col items-center justify-center gap-2 bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <h1 className="font-semibold">blurrd</h1>
      <p className="text-neutral-500">Blur player names in gameplay recordings.</p>
    </main>
  );
}
