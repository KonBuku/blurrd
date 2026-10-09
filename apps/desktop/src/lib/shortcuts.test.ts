import { describe, expect, it } from "vitest";

import { SHORTCUTS, ariaKeys, matchShortcut, shortcutText, targetOwnsKey } from "./shortcuts";

function key(init: KeyboardEventInit, target?: Element): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  if (!target) return event;
  let seen: KeyboardEvent | null = null;
  target.addEventListener("keydown", (e) => (seen = e as KeyboardEvent), { once: true });
  target.dispatchEvent(event);
  if (!seen) throw new Error("event not dispatched");
  return seen;
}

/** An element of the test document by id. */
function el(id: string): Element {
  return document.getElementById(id) ?? document.body;
}

describe("shortcuts", () => {
  it("matches bindings, alternatives and scopes", () => {
    expect(matchShortcut(key({ key: " ", code: "Space" }), "preview")?.id).toBe("playPause");
    expect(matchShortcut(key({ key: "K", code: "KeyK", shiftKey: false }), "preview")?.id).toBe(
      "playPause",
    );
    expect(matchShortcut(key({ key: "ArrowLeft", shiftKey: true }), "preview")?.id).toBe(
      "backSecond",
    );
    expect(matchShortcut(key({ key: "N", code: "KeyN", shiftKey: true }), "preview")?.id).toBe(
      "previousMark",
    );
    // Ctrl++ needs Shift on a US layout and not on a German one.
    expect(matchShortcut(key({ key: "+", ctrlKey: true, shiftKey: true }), "preview")?.id).toBe(
      "zoomIn",
    );
    expect(matchShortcut(key({ key: "+", ctrlKey: true }), "preview")?.id).toBe("zoomIn");
    expect(matchShortcut(key({ key: "o", code: "KeyO", ctrlKey: true }), "window")?.id).toBe(
      "addFiles",
    );
    expect(matchShortcut(key({ key: "o", code: "KeyO" }), "preview")?.id).toBe("toggleBoxes");
    expect(matchShortcut(key({ key: "o", code: "KeyO" }), "window")).toBeUndefined();
    expect(
      matchShortcut(key({ key: "Z", code: "KeyZ", ctrlKey: true, shiftKey: true }), "preview")?.id,
    ).toBe("redo");
    expect(matchShortcut(key({ key: "y", code: "KeyY", ctrlKey: true }), "preview")?.id).toBe(
      "redo",
    );
    expect(matchShortcut(key({ key: "z", code: "KeyZ", ctrlKey: true }), "preview")?.id).toBe(
      "undo",
    );
    expect(matchShortcut(key({ key: "ArrowUp", altKey: true }), "preview")?.id).toBe("nudgeBox");
    expect(
      matchShortcut(key({ key: "ArrowUp", altKey: true, shiftKey: true }), "preview")?.id,
    ).toBe("growBox");
    expect(matchShortcut(key({ key: "Backspace" }), "preview")?.id).toBe("deleteBox");
    expect(matchShortcut(key({ key: " " }), "window")).toBeUndefined();
  });

  it("repeats only frame steps, jumps and zoom", () => {
    expect(matchShortcut(key({ key: "ArrowRight", repeat: true }), "preview")?.id).toBe(
      "nextFrame",
    );
    expect(matchShortcut(key({ key: " ", repeat: true }), "preview")).toBeUndefined();
    expect(
      matchShortcut(key({ key: "b", code: "KeyB", ctrlKey: true, repeat: true }), "window"),
    ).toBeUndefined();
  });

  it("has one action per key combination", () => {
    const combos = Object.values(SHORTCUTS).flatMap((s) =>
      [s, ...s.alternatives].map(
        (b) => `${s.scope}:${b.ctrl}:${b.alt}:${String(b.shift)}:${b.key}`,
      ),
    );
    expect(new Set(combos).size).toBe(combos.length);
  });

  it("shows and announces keys", () => {
    expect(shortcutText(SHORTCUTS.playPause)).toBe("Space / K");
    expect(shortcutText(SHORTCUTS.addFolder)).toBe("Ctrl+Shift+O");
    expect(ariaKeys(SHORTCUTS.zoomIn)).toBe("Control+= Control+Plus");
    expect(ariaKeys(SHORTCUTS.previousFrame)).toBe("ArrowLeft");
  });

  it("leaves keys to the controls that use them", () => {
    document.body.innerHTML = `
      <input id="text" />
      <button id="button">Go</button>
      <div role="listbox" tabindex="0" id="list"></div>
      <div role="slider" tabindex="0" id="slider"></div>
      <div role="dialog"><span id="inDialog"></span></div>
      <div id="plain"></div>`;
    expect(targetOwnsKey(key({ key: "a" }, el("text")))).toBe(true);
    expect(targetOwnsKey(key({ key: "=", ctrlKey: true }, el("text")))).toBe(false);
    expect(targetOwnsKey(key({ key: " " }, el("button")))).toBe(true);
    expect(targetOwnsKey(key({ key: "k" }, el("button")))).toBe(false);
    expect(targetOwnsKey(key({ key: "ArrowDown" }, el("list")))).toBe(true);
    expect(targetOwnsKey(key({ key: "ArrowLeft" }, el("slider")))).toBe(true);
    expect(targetOwnsKey(key({ key: " " }, el("slider")))).toBe(false);
    expect(targetOwnsKey(key({ key: "a" }, el("inDialog")))).toBe(true);
    expect(targetOwnsKey(key({ key: "ArrowLeft" }, el("plain")))).toBe(false);
    // Text fields keep their own undo; the queue list its Delete.
    expect(targetOwnsKey(key({ key: "z", ctrlKey: true }, el("text")))).toBe(true);
    expect(targetOwnsKey(key({ key: "z", ctrlKey: true }, el("plain")))).toBe(false);
    expect(targetOwnsKey(key({ key: "Delete" }, el("list")))).toBe(true);
    expect(targetOwnsKey(key({ key: "Delete" }, el("plain")))).toBe(false);
  });
});
