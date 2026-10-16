// The IPC contract between the wrappers, the mock backend and the Tauri
// shell: every engine command a wrapper invokes is declared by the shell
// (`src-tauri/build.rs`), registered in its invoke handler
// (`src-tauri/src/lib.rs`), granted to the main window
// (`src-tauri/capabilities/default.json`) and answered by the mock backend,
// so the unit and end-to-end tests exercise the commands the app really
// has. Argument and result types are checked by tsc against the generated
// bindings.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const desktop = join(import.meta.dirname, "..", "..");
const read = (...path: string[]) => readFileSync(join(desktop, ...path), "utf8");

/** Every match of `pattern`'s first group in `text`, sorted. */
function matches(text: string, pattern: RegExp): string[] {
  return [...text.matchAll(pattern)].map((match) => match[1] ?? "").toSorted();
}

/** Engine commands the wrappers invoke. */
function wrapperCommands(): string[] {
  const sources = ["commands.ts", "jobs.ts", "presets.ts"].map((file) => read("src", "ipc", file));
  // Plugin commands ("plugin:…") do not match the pattern.
  return [
    ...new Set(sources.flatMap((text) => matches(text, /invoke<[^>]*>\(\s*"([a-z_]+)"/g))),
  ].toSorted();
}

describe("ipc contract", () => {
  const wrappers = wrapperCommands();

  it("finds the wrappers' commands", () => {
    expect(wrappers.length).toBeGreaterThan(20);
    expect(wrappers).toContain("start_analysis");
  });

  it("matches the commands the shell declares and registers", () => {
    const build = read("src-tauri", "build.rs");
    const declared = matches(build.slice(build.indexOf("const COMMANDS")), /^\s+"([a-z_]+)",$/gm);
    expect(declared).toEqual(wrappers);
    const lib = read("src-tauri", "src", "lib.rs");
    expect(matches(lib, /commands::([a-z_]+),/g)).toEqual(wrappers);
  });

  it("grants every command to the main window", () => {
    const capability = JSON.parse(read("src-tauri", "capabilities", "default.json")) as {
      permissions: unknown[];
    };
    const granted = capability.permissions
      .filter((permission): permission is string => typeof permission === "string")
      .filter((permission) => !permission.includes(":") && permission.startsWith("allow-"))
      .map((permission) => permission.slice("allow-".length).replaceAll("-", "_"))
      .toSorted();
    expect(granted).toEqual(wrappers);
  });

  it("is answered by the mock backend", () => {
    const mock = read("src", "ipc", "mock", "index.ts");
    const answered = new Set(matches(mock, /case "([a-z_]+)":/g));
    expect(wrappers.filter((command) => !answered.has(command))).toEqual([]);
  });

  it("takes the job arguments whose types are generated", () => {
    const commands = read("src-tauri", "src", "commands.rs");
    /** Argument names of command `name` as the frontend sends them. */
    const parameters = (name: string) => {
      const start = commands.indexOf(`fn ${name}(`);
      const list = commands.slice(start, commands.indexOf(") ->", start));
      return matches(list, /\n\s+([a-z_]+): /g)
        .filter((parameter) => parameter !== "state" && parameter !== "on_event")
        .map((parameter) => parameter.replaceAll(/_([a-z])/g, (_, c: string) => c.toUpperCase()))
        .toSorted();
    };
    /** Field names of a generated binding. */
    const fields = (type: string) => matches(read("src", "bindings", `${type}.ts`), /^(\w+): /gm);
    expect(parameters("start_analysis")).toEqual(fields("StartAnalysis"));
    expect(parameters("start_render")).toEqual(fields("StartRender"));
    expect(commands).toContain("files: Vec<RunFile>");
  });
});
