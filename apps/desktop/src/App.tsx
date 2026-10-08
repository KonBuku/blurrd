import { TitleBar } from "@/features/shell/TitleBar";

/**
 * The root component of the desktop app: the window shell with the title
 * bar on top and the inset content panel below it.
 */
export function App() {
  return (
    <div className="flex h-full flex-col bg-background-win-alt text-foreground">
      <TitleBar />
      <main className="mx-1 mb-1 flex min-h-0 flex-1 flex-col items-center justify-center gap-1 rounded-[5px] border border-border bg-background">
        <h1 className="text-ui-lg font-medium">blurrd</h1>
        <p className="text-ui-sm text-foreground-subtle">
          Blur player names in gameplay recordings.
        </p>
      </main>
    </div>
  );
}
