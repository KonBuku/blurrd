import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { REGION_ORDER, REGIONS } from "@/lib/regions";

import { NumberField, normalizeNumber } from "./number-field";
import { RegionChip } from "./region-chip";
import { Slider } from "./slider";
import { ToggleGroup } from "./toggle-group";

/** A slider that owns its value, for the reset test. */
function ControlledSlider() {
  const [value, setValue] = useState([14]);
  return <Slider aria-label="Strength" value={value} onValueChange={setValue} resetValue={[10]} />;
}

/** A padding field that owns its value and reports every change. */
function Padding({ onChange }: { onChange: (value: number) => void }) {
  const [value, setValue] = useState(4);
  return (
    <NumberField
      aria-label="Padding"
      value={value}
      min={0}
      max={32}
      unit="px"
      onValueChange={(next) => {
        setValue(next);
        onChange(next);
      }}
    />
  );
}

describe("Slider", () => {
  it("names its thumb and steps with the keyboard", () => {
    const onValueChange = vi.fn<(value: number[]) => void>();
    render(
      <Slider
        aria-label="Strength"
        defaultValue={[10]}
        min={8}
        max={20}
        onValueChange={onValueChange}
      />,
    );
    const thumb = screen.getByRole("slider", { name: "Strength" });
    expect(thumb.getAttribute("aria-valuenow")).toBe("10");
    fireEvent.keyDown(thumb, { key: "ArrowRight" });
    expect(onValueChange).toHaveBeenLastCalledWith([11]);
  });

  it("restores the reset value on double-click", () => {
    render(<ControlledSlider />);
    const thumb = screen.getByRole("slider", { name: "Strength" });
    fireEvent.doubleClick(thumb);
    expect(thumb.getAttribute("aria-valuenow")).toBe("10");
  });
});

describe("ToggleGroup", () => {
  const options = [
    { value: "low", label: "Low" },
    { value: "medium", label: "Medium" },
    { value: "high", label: "High" },
  ] as const;

  function Strength({ onChange }: { onChange: (value: string) => void }) {
    const [value, setValue] = useState<(typeof options)[number]["value"]>("medium");
    return (
      <ToggleGroup
        aria-label="Strength"
        options={options}
        value={value}
        onValueChange={(next) => {
          setValue(next);
          onChange(next);
        }}
      />
    );
  }

  it("selects one option at a time", () => {
    const onChange = vi.fn<(value: string) => void>();
    render(<Strength onChange={onChange} />);
    const high = screen.getByRole("radio", { name: "High" });
    fireEvent.click(high);
    expect(onChange).toHaveBeenLastCalledWith("high");
    expect(high.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: "Medium" }).getAttribute("aria-checked")).toBe(
      "false",
    );
  });

  it("keeps the active option when it is clicked again", () => {
    const onChange = vi.fn<(value: string) => void>();
    render(<Strength onChange={onChange} />);
    const medium = screen.getByRole("radio", { name: "Medium" });
    fireEvent.click(medium);
    expect(onChange).not.toHaveBeenCalled();
    expect(medium.getAttribute("aria-checked")).toBe("true");
  });
});

describe("NumberField", () => {
  it("steps with the arrow keys, ten times with Shift, within the limits", () => {
    const onChange = vi.fn<(value: number) => void>();
    render(<Padding onChange={onChange} />);
    const field = screen.getByRole("spinbutton", { name: "Padding" });
    fireEvent.keyDown(field, { key: "ArrowUp" });
    expect(field.getAttribute("aria-valuenow")).toBe("5");
    fireEvent.keyDown(field, { key: "ArrowUp", shiftKey: true });
    expect(field.getAttribute("aria-valuenow")).toBe("15");
    fireEvent.keyDown(field, { key: "ArrowUp", shiftKey: true });
    fireEvent.keyDown(field, { key: "ArrowUp", shiftKey: true });
    expect(field.getAttribute("aria-valuenow")).toBe("32");
    fireEvent.keyDown(field, { key: "Home" });
    expect(onChange).toHaveBeenLastCalledWith(0);
  });

  it("commits typed numbers on Enter and reverts invalid text on blur", () => {
    const onChange = vi.fn<(value: number) => void>();
    render(<Padding onChange={onChange} />);
    const field = screen.getByRole<HTMLInputElement>("spinbutton", { name: "Padding" });
    fireEvent.change(field, { target: { value: "12" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(onChange).toHaveBeenLastCalledWith(12);

    fireEvent.change(field, { target: { value: "abc" } });
    fireEvent.blur(field);
    expect(field.value).toBe("12");
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("normalizes values to the step and the limits", () => {
    expect(normalizeNumber(0.3333, 0, 1, 0.05)).toBe(0.35);
    expect(normalizeNumber(-4, 0, 10, 1)).toBe(0);
    expect(normalizeNumber(7.6, 0, 10, 1)).toBe(8);
  });
});

describe("RegionChip", () => {
  it("shows the label of every region", () => {
    render(
      <>
        {REGION_ORDER.map((region) => (
          <RegionChip key={region} region={region} />
        ))}
      </>,
    );
    for (const region of REGION_ORDER) {
      expect(screen.getByText(REGIONS[region].label).getAttribute("data-region")).toBe(region);
    }
  });

  it("gives the four screen surfaces one colour family", () => {
    const families = new Set(
      REGION_ORDER.filter((region) => REGIONS[region].group === "screens").map(
        (region) => REGIONS[region].family,
      ),
    );
    expect([...families]).toEqual(["screens"]);
  });

  it("uses the family colour classes", () => {
    render(<RegionChip region="DeathRecap" />);
    expect(screen.getByText("Death recap").className).toContain("text-region-screens");
  });
});
