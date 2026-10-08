import { Copy, FolderOpen, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { GpuIcon } from "@/components/icons/processing";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

import { Section } from "./Section";

const QUEUE = Array.from(
  { length: 12 },
  (_, index) => `clip_${String(index + 1).padStart(2, "0")}.mp4`,
);

/** Tabs, progress, cards, scroll areas and the floating layers. */
export function FeedbackSections() {
  return (
    <>
      <Section title="Tabs">
        <Tabs defaultValue="blur" className="w-96">
          <TabsList aria-label="Inspector">
            <TabsTrigger value="blur">Blur</TabsTrigger>
            <TabsTrigger value="export">Export</TabsTrigger>
            <TabsTrigger value="verify">Verify</TabsTrigger>
          </TabsList>
          <TabsContent value="blur" className="text-ui-sm text-foreground-subtle">
            Regions, style and strength.
          </TabsContent>
          <TabsContent value="export" className="text-ui-sm text-foreground-subtle">
            Codec, encoder and output folder.
          </TabsContent>
          <TabsContent value="verify" className="text-ui-sm text-foreground-subtle">
            OCR verification of the output.
          </TabsContent>
        </Tabs>
      </Section>

      <Section title="Progress and loading" className="flex-col items-stretch">
        <div className="flex max-w-md items-center gap-3">
          <Progress aria-label="Export progress" value={62} />
          <span className="shrink-0 font-mono text-ui-sm text-foreground-subtle tabular-nums">
            62 %
          </span>
        </div>
        <div className="flex max-w-md items-center gap-3">
          <Progress aria-label="Probing the GPU" value={null} />
          <Spinner label="Probing" className="text-foreground-subtle" />
        </div>
      </Section>

      <Section title="Cards and scroll area" className="items-stretch">
        <Card className="w-72">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <GpuIcon />
              GPU
            </CardTitle>
            <CardDescription>NVIDIA GeForce RTX 4080 SUPER</CardDescription>
          </CardHeader>
          <CardContent className="grid grid-cols-3 gap-2">
            {[
              ["Decode", "NVDEC"],
              ["Encode", "NVENC"],
              ["Sessions", "2"],
            ].map(([label, value]) => (
              <div key={label} className="flex flex-col gap-0.5">
                <span className="text-ui-sm text-foreground-subtle">{label}</span>
                <span className="text-ui-lg font-medium">{value}</span>
              </div>
            ))}
          </CardContent>
          <CardFooter>
            <Button variant="secondary" size="sm">
              Run benchmark
            </Button>
          </CardFooter>
        </Card>
        <ScrollArea className="h-44 w-60 rounded-xl border border-border bg-panel">
          <ul className="flex flex-col gap-0.5 p-1">
            {QUEUE.map((name) => (
              <li
                key={name}
                className="flex h-8 items-center rounded-lg px-2.5 text-ui-base hover:bg-surface-hover"
              >
                {name}
              </li>
            ))}
          </ul>
        </ScrollArea>
      </Section>

      <Section title="Menus, dialogs, tooltips and toasts">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline">Queue item</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-56">
            <DropdownMenuLabel>clip_01.mp4</DropdownMenuLabel>
            <DropdownMenuGroup>
              <DropdownMenuItem>
                <FolderOpen />
                Show in folder
                <DropdownMenuShortcut>Ctrl+E</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem>
                <Copy />
                Copy path
              </DropdownMenuItem>
              <DropdownMenuCheckboxItem checked>Verify after export</DropdownMenuCheckboxItem>
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive">
              <Trash2 />
              Remove from queue
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <Dialog>
          <DialogTrigger asChild>
            <Button variant="outline">Verify report</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Verify report</DialogTitle>
              <DialogDescription>
                OCR found no readable names in the exported video.
              </DialogDescription>
            </DialogHeader>
            <Separator />
            <DialogFooter>
              <DialogClose asChild>
                <Button variant="secondary">Close</Button>
              </DialogClose>
              <Button>Accept</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="outline">Command palette</Button>
          </TooltipTrigger>
          <TooltipContent>
            Open the command palette
            <Kbd>Ctrl K</Kbd>
          </TooltipContent>
        </Tooltip>

        <Button
          variant="outline"
          onClick={() =>
            toast.success("Export finished", { description: "clip_01_blurred.mp4 is ready." })
          }
        >
          Show toast
        </Button>
      </Section>
    </>
  );
}
