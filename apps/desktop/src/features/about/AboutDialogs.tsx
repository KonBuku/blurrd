import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useReturnFocus } from "@/hooks/useReturnFocus";

import { AboutContent } from "./AboutContent";
import { LicensesDialog } from "./LicensesDialog";
import { useAboutStore } from "./store";

/**
 * The About dialog and the licences dialog as the command palette opens
 * them (without a trigger, so focus returns to where it was).
 */
export function AboutDialogs() {
  const open = useAboutStore((state) => state.open);
  const close = useAboutStore((state) => state.close);
  const aboutFocus = useReturnFocus();
  const licensesFocus = useReturnFocus();
  const onOpenChange = (next: boolean) => {
    if (!next) close();
  };

  return (
    <>
      <Dialog open={open === "about"} onOpenChange={onOpenChange}>
        <DialogContent className="gap-0 sm:max-w-lg" {...aboutFocus}>
          <DialogDescription className="sr-only">
            Version, licences and trademark statements of blurrd.
          </DialogDescription>
          <AboutContent
            heading={<DialogTitle className="text-ui-lg font-semibold">blurrd</DialogTitle>}
          />
        </DialogContent>
      </Dialog>
      <LicensesDialog
        open={open === "licenses"}
        onOpenChange={onOpenChange}
        contentProps={licensesFocus}
      />
    </>
  );
}
