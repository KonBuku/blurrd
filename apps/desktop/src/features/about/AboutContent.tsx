import brandIconLight from "@brand/blurrd-icon-light.svg";
import brandIcon from "@brand/blurrd-icon.svg";
import { Bug, ExternalLink, ScrollText } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { useCapabilities } from "@/features/gpu/useCapabilities";

import { ffmpegSource } from "./licenses";
import { LicensesDialog, SourceLink } from "./LicensesDialog";
import { ISSUES_URL, openProjectPage, REPOSITORY_URL } from "./links";
import { useLicenses } from "./useLicenses";

/** Version and commit of this build (set by `vite.config.ts`). */
export const BUILD = {
  version: import.meta.env.APP_VERSION,
  commit: import.meta.env.APP_COMMIT,
} as const;

/** Trademark statements (plan 14: README "Legal" and About). */
export const TRADEMARKS =
  "blurrd is an independent open-source project and is not affiliated with, endorsed, sponsored or approved by Riot Games, Inc. VALORANT and Riot Games are trademarks or registered trademarks of Riot Games, Inc. NVIDIA and NVENC are trademarks of NVIDIA Corporation. FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.";

function Fact({ term, children }: { readonly term: string; readonly children: ReactNode }) {
  return (
    <>
      <dt className="text-foreground-subtle">{term}</dt>
      <dd className="flex min-w-0 flex-wrap items-center gap-x-3">{children}</dd>
    </>
  );
}

export interface AboutContentProps {
  /** The product name as the surrounding view's heading (dialog title or page heading). */
  readonly heading: ReactNode;
}

/**
 * What the About dialog and Settings › About show (design 8.6): the mark,
 * version and commit, the FFmpeg the app runs with and where its source
 * is, the project links, the open-source licences and the trademark
 * statements.
 */
export function AboutContent({ heading }: AboutContentProps) {
  const ffmpegVersion = useCapabilities().data?.ffmpeg_version ?? null;
  const licenses = useLicenses();
  const source = licenses.data ? ffmpegSource(licenses.data) : null;
  const [licensesOpen, setLicensesOpen] = useState(false);

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-center gap-3">
        <img src={brandIcon} alt="" className="size-12 rounded-xl dark:hidden" />
        <img src={brandIconLight} alt="" className="hidden size-12 rounded-xl dark:block" />
        <div className="flex min-w-0 flex-col gap-0.5">
          {heading}
          <p className="text-ui-base text-foreground-subtle">
            Blurs player names in gameplay recordings, on your PC.
          </p>
        </div>
      </div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-1.5 text-ui-base">
        <Fact term="Version">
          <span className="font-mono">{BUILD.version}</span>
        </Fact>
        <Fact term="Commit">
          <span className="font-mono">{BUILD.commit}</span>
        </Fact>
        <Fact term="FFmpeg">
          <span className="font-mono">{ffmpegVersion ?? "Not detected yet"}</span>
          {source !== null && <SourceLink url={source}>Source code</SourceLink>}
        </Fact>
        <Fact term="Licence">GNU GPL 3.0 only</Fact>
      </dl>
      <p className="text-ui-base">
        Built with Tauri, React, FFmpeg and ONNX Runtime, and many other open-source projects.
      </p>
      <div className="flex flex-wrap gap-2">
        <LicensesDialog
          open={licensesOpen}
          onOpenChange={setLicensesOpen}
          trigger={
            <Button variant="outline" size="lg">
              <ScrollText aria-hidden />
              Open-source licenses
            </Button>
          }
        />
        <Button variant="ghost" size="lg" onClick={() => void openProjectPage(REPOSITORY_URL)}>
          <ExternalLink aria-hidden />
          GitHub
        </Button>
        <Button variant="ghost" size="lg" onClick={() => void openProjectPage(ISSUES_URL)}>
          <Bug aria-hidden />
          Report an issue
        </Button>
      </div>
      <p className="text-ui-sm text-foreground-subtle">{TRADEMARKS}</p>
    </div>
  );
}
