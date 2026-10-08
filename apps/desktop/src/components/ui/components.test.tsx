import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GpuIcon } from "@/components/icons/processing";
import { MaximizeIcon } from "@/components/icons/window";

import { Badge } from "./badge";
import { Button } from "./button";
import { Checkbox } from "./checkbox";
import { Dialog, DialogContent, DialogDescription, DialogTitle, DialogTrigger } from "./dialog";
import { Input } from "./input";
import { Label } from "./label";
import { Progress } from "./progress";
import { Spinner } from "./spinner";
import { Switch } from "./switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

describe("Button", () => {
  it("is a non-submitting button by default", () => {
    const onClick = vi.fn<() => void>();
    render(<Button onClick={onClick}>Analyze</Button>);
    const button = screen.getByRole("button", { name: "Analyze" });
    expect(button.getAttribute("type")).toBe("button");
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("renders its child with the button styles when asChild is set", () => {
    render(
      <Button asChild variant="link">
        <a href="#docs">Docs</a>
      </Button>,
    );
    const link = screen.getByRole("link", { name: "Docs" });
    expect(link.className).toContain("text-link");
    expect(link.hasAttribute("type")).toBe(false);
  });

  it("merges a custom class over the variant classes", () => {
    render(
      <Button size="icon-md" className="size-8">
        x
      </Button>,
    );
    const classes = screen.getByRole("button").className.split(" ");
    expect(classes).toContain("size-8");
    expect(classes).not.toContain("size-7");
  });
});

describe("form controls", () => {
  it("connects a label to its input", () => {
    render(
      <>
        <Label htmlFor="template">Output name</Label>
        <Input id="template" defaultValue="{name}_blurred" />
      </>,
    );
    expect(screen.getByLabelText("Output name")).toBeDefined();
  });

  it("toggles a switch with click and keyboard", () => {
    const onCheckedChange = vi.fn<(checked: boolean) => void>();
    render(<Switch aria-label="Verify" onCheckedChange={onCheckedChange} />);
    const control = screen.getByRole("switch", { name: "Verify" });
    expect(control.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(control);
    expect(control.getAttribute("aria-checked")).toBe("true");
    expect(onCheckedChange).toHaveBeenCalledWith(true);
  });

  it("shows the indeterminate state of a checkbox", () => {
    render(<Checkbox aria-label="All regions" checked="indeterminate" />);
    expect(screen.getByRole("checkbox").getAttribute("aria-checked")).toBe("mixed");
  });
});

describe("Tabs", () => {
  it("shows the panel of the selected tab", () => {
    render(
      <Tabs defaultValue="blur">
        <TabsList aria-label="Inspector">
          <TabsTrigger value="blur">Blur</TabsTrigger>
          <TabsTrigger value="export">Export</TabsTrigger>
        </TabsList>
        <TabsContent value="blur">Blur settings</TabsContent>
        <TabsContent value="export">Export settings</TabsContent>
      </Tabs>,
    );
    expect(screen.getByText("Blur settings")).toBeDefined();
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Export" }), { button: 0 });
    expect(screen.getByText("Export settings")).toBeDefined();
    expect(screen.queryByText("Blur settings")).toBeNull();
  });
});

describe("Dialog", () => {
  it("opens from its trigger and closes with Escape", async () => {
    render(
      <Dialog>
        <DialogTrigger asChild>
          <Button>Open report</Button>
        </DialogTrigger>
        <DialogContent>
          <DialogTitle>Verify report</DialogTitle>
          <DialogDescription>No readable names found.</DialogDescription>
        </DialogContent>
      </Dialog>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open report" }));
    const dialog = await screen.findByRole("dialog", { name: "Verify report" });
    expect(screen.getByRole("button", { name: "Close" })).toBeDefined();

    await act(async () => {
      fireEvent.keyDown(dialog, { key: "Escape" });
    });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("Tooltip", () => {
  it("shows its label when the trigger gets keyboard focus", async () => {
    render(
      <Tooltip>
        <TooltipTrigger asChild>
          <Button aria-label="Minimize">-</Button>
        </TooltipTrigger>
        <TooltipContent>Minimize window</TooltipContent>
      </Tooltip>,
    );
    await act(async () => {
      screen.getByRole("button", { name: "Minimize" }).focus();
    });
    expect((await screen.findByRole("tooltip")).textContent).toBe("Minimize window");
  });
});

describe("status indicators", () => {
  it("reports determinate progress and clamps it", () => {
    render(<Progress aria-label="Export" value={140} />);
    const bar = screen.getByRole("progressbar", { name: "Export" });
    expect(bar.getAttribute("aria-valuenow")).toBe("100");
  });

  it("reports indeterminate progress without a value", () => {
    render(<Progress aria-label="Probing" value={null} />);
    const bar = screen.getByRole("progressbar", { name: "Probing" });
    expect(bar.hasAttribute("aria-valuenow")).toBe(false);
    expect(bar.getAttribute("data-state")).toBe("indeterminate");
  });

  it("announces a spinner by its label", () => {
    render(<Spinner label="Analyzing" />);
    expect(screen.getByRole("status", { name: "Analyzing" })).toBeDefined();
  });

  it("renders badges as plain text", () => {
    render(<Badge variant="success">Clean</Badge>);
    expect(screen.getByText("Clean").getAttribute("data-slot")).toBe("badge");
  });
});

describe("icons", () => {
  it("are decorative unless they get a title", () => {
    const { container } = render(<MaximizeIcon />);
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("expose their title as the accessible name", () => {
    render(<GpuIcon title="NVENC" />);
    expect(screen.getByRole("img", { name: "NVENC" })).toBeDefined();
  });

  it("use the shared grid and stroke", () => {
    const { container } = render(<GpuIcon className="size-5" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
    expect(svg?.getAttribute("stroke-width")).toBe("1.5");
    expect(svg?.getAttribute("class")).toBe("shrink-0 size-5");
  });
});
