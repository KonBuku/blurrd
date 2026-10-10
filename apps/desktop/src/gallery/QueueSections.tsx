import { QueueRow } from "@/features/queue/QueueRow";
import { fakeMedia } from "@/ipc/mock/media";
import { pathKey } from "@/lib/media";
import type { QueueStatus } from "@/lib/queue-status";
import type { QueueItem } from "@/store/queue";

import { Section } from "./Section";

/** One queue item per status of design 8.1, plus the UNVERIFIED state. */
const STATUSES: readonly QueueStatus[] = [
  { kind: "queued" },
  { kind: "probing", folder: false },
  { kind: "analyzing", progress: 0.46, fps: 820 },
  { kind: "ready", boxes: 1284 },
  { kind: "exporting", progress: 0.63, fps: 388, etaMs: 21_000 },
  { kind: "waiting", prior: { kind: "ready", boxes: 312 } },
  { kind: "verifying", pass: 2, passes: 3 },
  { kind: "done", verified: true },
  { kind: "done", verified: false },
  { kind: "warnings", leaks: 2 },
  { kind: "unverified" },
  { kind: "failed", reason: "Decoder error", detail: null },
  { kind: "canceled" },
];

const ITEMS: readonly QueueItem[] = STATUSES.map((status, index) => {
  const path = `D:\\Recordings\\match_${String(index + 1).padStart(2, "0")}.mp4`;
  return {
    id: pathKey(path),
    path,
    name: `match_${String(index + 1).padStart(2, "0")}.mp4`,
    media: status.kind === "probing" ? null : fakeMedia(path),
    status,
    output: null,
    added: index + 1,
  };
});

/** Row callbacks of the static specimens. */
function ignore() {
  // The gallery rows are not interactive.
}

/** Queue rows in every status, on the shell colour like the sidebar. */
export function QueueSections() {
  return (
    <Section title="Queue statuses">
      <div
        // The rows are listbox options; the gallery shows them statically.
        // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
        role="listbox"
        aria-label="Queue statuses"
        className="grid w-full grid-cols-2 gap-x-3 gap-y-0.5 rounded-xl bg-background-win-alt p-2"
      >
        {ITEMS.map((item, index) => (
          <QueueRow
            key={item.id}
            item={item}
            selected={index === 3}
            active={false}
            onPick={ignore}
            onContext={ignore}
            onRemove={ignore}
          />
        ))}
      </div>
    </Section>
  );
}
