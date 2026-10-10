import type { ReactNode } from "react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { useJobSettingsStore, type ApplyScope } from "@/store/job-settings";
import { useUiStore, type InspectorTab } from "@/store/ui";

import { BlurInspector } from "./BlurInspector";
import { ExportFooter } from "./ExportFooter";
import { ExportInspector } from "./ExportInspector";
import { PresetBar } from "./PresetBar";
import { VerifyInspector } from "./VerifyInspector";
import { useInspectorTarget } from "./useInspector";

/** Id of the inspector frame, referenced by its toggle and resize handle. */
export const INSPECTOR_ID = "inspector";

/** The inspector tabs. */
const TABS: readonly { value: InspectorTab; label: string }[] = [
  { value: "blur", label: "Blur" },
  { value: "export", label: "Export" },
  { value: "verify", label: "Verify" },
];

function isInspectorTab(value: string): value is InspectorTab {
  return TABS.some((tab) => tab.value === value);
}

export interface InspectorTabsProps {
  /** Shown above the tab list (the sheet's title row). */
  readonly header?: ReactNode;
}

/**
 * The inspector content: a segmented Blur | Export | Verify tab list, the
 * preset and "Apply to" rows, the selected tab and the export footer.
 * The same content fills the inspector frame and, in narrow windows, the
 * inspector sheet; the selected tab is shared.
 */
export function InspectorTabs({ header }: InspectorTabsProps) {
  const tab = useUiStore((state) => state.inspectorTab);
  const setTab = useUiStore((state) => state.setInspectorTab);

  return (
    <Tabs
      value={tab}
      onValueChange={(value) => {
        if (isInspectorTab(value)) setTab(value);
      }}
      className="min-h-0 flex-1 gap-0"
    >
      {header}
      <div className="flex h-10 shrink-0 items-center border-b border-border px-1.5">
        <TabsList aria-label="Inspector" className="w-full">
          {TABS.map(({ value, label }) => (
            <TabsTrigger key={value} value={value} className="flex-1">
              {label}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      <PresetBar />
      <ApplyScopeBar />
      {TABS.map(({ value }) => (
        <TabsContent key={value} value={value} className="overflow-y-auto p-3">
          {value === "blur" && <BlurInspector />}
          {value === "export" && <ExportInspector />}
          {value === "verify" && <VerifyInspector />}
        </TabsContent>
      ))}
      <ExportFooter />
    </Tabs>
  );
}

/** What an inspector change applies to, in words. */
function targetText(
  scope: ApplyScope,
  lead: { readonly name: string } | null,
  fileCount: number,
): string {
  if (scope === "all") {
    return fileCount === 0 ? "Defaults for new files" : `All ${fileCount} files and new files`;
  }
  if (fileCount > 1) return `${fileCount} selected files`;
  return lead ? lead.name : "Defaults for new files";
}

/**
 * The inspector's target line: whose settings are shown and the "Apply
 * to" switch between the selected files and every file (design 8.1).
 */
function ApplyScopeBar() {
  const { lead, fileCount } = useInspectorTarget();
  const scope = useJobSettingsStore((state) => state.scope);
  const setScope = useJobSettingsStore((state) => state.setScope);
  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
      <p
        data-testid="inspector-target"
        className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtle"
      >
        {targetText(scope, lead, fileCount)}
      </p>
      <ToggleGroup<ApplyScope>
        size="sm"
        aria-label="Apply changes to"
        value={scope}
        options={[
          { value: "selected", label: "Selected" },
          { value: "all", label: "All" },
        ]}
        onValueChange={setScope}
      />
    </div>
  );
}
