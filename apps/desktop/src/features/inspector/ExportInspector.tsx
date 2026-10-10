import type { KeyframePolicy } from "@/bindings/KeyframePolicy";
import type { MoovMode } from "@/bindings/MoovMode";
import type { Toggle } from "@/bindings/Toggle";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { useCapabilities } from "@/features/gpu/useCapabilities";

import { fieldSetter, type ExportChange } from "./export-section";
import { AudioSection, FileSection } from "./ExportFileSections";
import { VideoSection } from "./ExportVideoSection";
import { OutputEstimate } from "./OutputEstimate";
import { Disclosure, InspectorRow, InspectorSection } from "./parts";
import { useInspectorTarget, useLeadProject } from "./useInspector";

/**
 * Inspector › Export (design 8.1): video, audio and file settings, the
 * output name with the size and time estimate, and the advanced encoding
 * options. Every change goes to the inspector's targets.
 */
export function ExportInspector() {
  const { lead, update, settings } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const caps = useCapabilities().data ?? null;
  const output = settings.export;
  // The opened project knows the decoded stream; the queue's probe until then.
  const media = project?.media ?? lead?.media ?? null;
  const change: ExportChange = (patch) => update("export", patch);
  const set = fieldSetter(change);

  return (
    <div className="flex flex-col gap-4">
      <VideoSection output={output} change={change} caps={caps} media={media} />
      <AudioSection output={output} change={change} />
      <FileSection output={output} change={change} />
      <OutputEstimate project={project ?? null} settings={output} />
      <Disclosure label="Advanced export">
        <InspectorSection title="Encoding">
          <InspectorRow
            label="Split encoding"
            description="Encodes HEVC and AV1 frames in two halves on GPUs that support it."
          >
            <ToggleGroup<Toggle>
              size="sm"
              aria-label="Split encoding"
              value={output.split_encode}
              options={[
                { value: "Auto", label: "Auto" },
                { value: "On", label: "On" },
                { value: "Off", label: "Off" },
              ]}
              onValueChange={(split) => set("split_encode", split)}
            />
          </InspectorRow>
          <InspectorRow
            label="Keyframes"
            description="Keeping the source's keyframes keeps seeking and cutting the same as in the original."
          >
            <ToggleGroup<KeyframePolicy>
              size="sm"
              aria-label="Keyframes"
              value={output.keyframes}
              options={[
                { value: "MatchSource", label: "Match source" },
                { value: "EncoderDefault", label: "Encoder default" },
              ]}
              onValueChange={(keyframes) => set("keyframes", keyframes)}
            />
          </InspectorRow>
          <InspectorRow
            label="MP4 and MOV index"
            description="Where the index goes; at the start, players can begin before the file has loaded."
          >
            <ToggleGroup<MoovMode>
              size="sm"
              aria-label="MP4 and MOV index"
              value={output.moov}
              options={[
                { value: "Reserve", label: "Start" },
                { value: "Faststart", label: "Start, rewritten" },
                { value: "AtEnd", label: "End" },
              ]}
              onValueChange={(moov) => set("moov", moov)}
            />
          </InspectorRow>
        </InspectorSection>
      </Disclosure>
    </div>
  );
}
