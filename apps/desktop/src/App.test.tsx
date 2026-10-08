import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { App } from "./App";
import { installMockBackend } from "./ipc/mock";

describe("App", () => {
  beforeEach(() => {
    installMockBackend();
  });

  it("renders the title bar and the product name", () => {
    render(<App />);
    expect(screen.getByRole("banner")).toBeDefined();
    expect(screen.getByRole("heading", { name: "blurrd" })).toBeDefined();
  });
});
