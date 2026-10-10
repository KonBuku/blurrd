import { AboutContent } from "@/features/about/AboutContent";

import { SettingsPage } from "./SettingsRow";

/** Settings › About: the About dialog's content as a page (design 8.5, 8.6). */
export function AboutSettings() {
  return (
    <SettingsPage title="About">
      <div className="rounded-xl border border-card-border bg-card p-4">
        <AboutContent heading={<h2 className="text-ui-lg font-semibold">blurrd</h2>} />
      </div>
    </SettingsPage>
  );
}
