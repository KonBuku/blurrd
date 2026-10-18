import { useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { GpuStatusCard } from "@/features/gpu/GpuStatusCard";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { toAppError } from "@/ipc/errors";
import { refreshCapabilities } from "@/ipc/settings";
import { needsRestart, usesGpu } from "@/lib/app-defaults";
import { useSettingsStore } from "@/store/settings";

import { ReadOnlyNotice, SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** Select value of "the high-performance GPU" (no LUID stored). */
const AUTO_GPU = "auto";

/**
 * Settings › Processing (plan 6.3 `processing`): this PC's GPU, decoders,
 * encoders and OCR; whether the GPU decodes and encodes; the GPU the OCR
 * verification runs on; and measuring the hardware again. Only what the
 * engine reads: jobs always run one at a time, and the encoder and NVENC
 * sessions are export settings (Output defaults, inspector).
 */
export function ProcessingSettings() {
  const processing = useSettingsStore((state) => state.processing);
  const active = useSettingsStore((state) => state.activeProcessing);
  const update = useSettingsStore((state) => state.updateProcessing);
  const caps = useCapabilities().data ?? null;
  const queryClient = useQueryClient();
  const [measuring, setMeasuring] = useState(false);
  const gpuLabel = useId();
  const adapterLabel = useId();
  const measureLabel = useId();
  const gpus = caps?.gpus ?? [];
  const probing = caps === null || caps.probe_state === "Pending";
  // A stored GPU that is not installed any more is still shown, so the
  // select never pretends that the automatic choice is stored.
  const storedMissing =
    processing.gpu_luid !== null && !gpus.some((gpu) => gpu.luid === processing.gpu_luid);

  const measure = async () => {
    setMeasuring(true);
    try {
      await refreshCapabilities();
      await queryClient.invalidateQueries({ queryKey: ["capabilities"] });
    } catch (error) {
      toast.error("The hardware could not be measured again", {
        description: toAppError(error).message,
      });
    } finally {
      setMeasuring(false);
    }
  };

  return (
    <SettingsPage title="Processing" description="How blurrd uses the hardware of this PC.">
      <ReadOnlyNotice />
      {needsRestart(processing, active) && (
        <p aria-live="polite" className="text-ui-base text-warning">
          Restart blurrd to apply the changes below; it keeps using the previous choice until then.
        </p>
      )}
      <GpuStatusCard variant="full" />
      <SettingsGroup>
        <SettingsRow
          label="Use the GPU"
          labelId={gpuLabel}
          description="Decode and encode on the graphics card when it can. When off, blurrd decodes on the CPU and encodes with the Windows encoders, which is much slower."
          control={
            <Switch
              aria-labelledby={gpuLabel}
              checked={usesGpu(processing)}
              onCheckedChange={(on) => update({ hw_decode: on ? "Auto" : "Software" })}
            />
          }
        />
        <SettingsRow
          label="GPU for text recognition"
          labelId={adapterLabel}
          description={
            gpus[0]
              ? `The graphics card the OCR verification runs on. Automatic uses the high-performance GPU, here the ${gpus[0].name}.`
              : "The graphics card the OCR verification runs on. Automatic uses the high-performance GPU."
          }
          controlWidth="wide"
          control={
            <Select
              value={processing.gpu_luid ?? AUTO_GPU}
              disabled={probing && processing.gpu_luid === null}
              onValueChange={(value) => update({ gpu_luid: value === AUTO_GPU ? null : value })}
            >
              <SelectTrigger
                size="lg"
                aria-labelledby={adapterLabel}
                className="w-full min-w-0 [&>span]:truncate"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={AUTO_GPU}>Automatic</SelectItem>
                {gpus.map((gpu) => (
                  <SelectItem key={gpu.luid} value={gpu.luid}>
                    {gpu.name}
                  </SelectItem>
                ))}
                {storedMissing && processing.gpu_luid !== null && (
                  <SelectItem value={processing.gpu_luid}>GPU not found</SelectItem>
                )}
              </SelectContent>
            </Select>
          }
        />
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow
          label="Measure the hardware again"
          labelId={measureLabel}
          description="blurrd tests the encoders and measures the decoders of this PC once and keeps the result. Measure again after changing the graphics card or its driver."
          control={
            <Button
              variant="outline"
              size="lg"
              aria-describedby={measureLabel}
              disabled={measuring || probing}
              onClick={() => void measure()}
            >
              <RefreshCw
                aria-hidden
                className={measuring || probing ? "animate-spin motion-reduce:animate-none" : ""}
              />
              {probing ? "Measuring…" : "Measure again"}
            </Button>
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
