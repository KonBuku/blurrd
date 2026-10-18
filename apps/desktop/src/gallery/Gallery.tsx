import { QueryClientProvider } from "@tanstack/react-query";

import { Toaster } from "@/components/ui/toaster";
import { queryClient } from "@/lib/query";

import { ControlSections } from "./ControlSections";
import { FeedbackSections } from "./FeedbackSections";
import { FoundationSections } from "./FoundationSections";
import { QueueSections } from "./QueueSections";

/**
 * Every design token and component of the desktop app on one page, for the
 * Playwright screenshot and focus tests and for reviewing the look in a
 * browser (`pnpm dev:mock`, then open `/?gallery`). Development only:
 * `main.tsx` loads it in the `mock` mode, so production builds never
 * contain it. The theme is the app's (dark unless the settings say otherwise).
 */
export function Gallery() {
  return (
    // Queue rows load their posters from the mock scheme.
    <QueryClientProvider client={queryClient}>
      <GalleryPage />
    </QueryClientProvider>
  );
}

function GalleryPage() {
  return (
    <div className="min-h-full bg-background-win-alt p-1">
      <main className="mx-auto flex max-w-5xl flex-col gap-8 rounded-[5px] border border-border bg-background px-8 py-6">
        <header className="flex flex-col gap-1">
          <h1 className="text-ui-xl font-semibold">Component gallery</h1>
          <p className="text-ui-sm text-foreground-subtle">
            Design tokens and components of the blurrd desktop app.
          </p>
        </header>
        <FoundationSections />
        <ControlSections />
        <FeedbackSections />
        <QueueSections />
      </main>
      <Toaster />
    </div>
  );
}
