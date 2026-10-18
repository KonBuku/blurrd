/**
 * Checks that a licence text matches the licence it is listed under, so a
 * wrong file behind a licence (such as blurrd's own GPL LICENSE behind
 * "Apache-2.0") stops `pnpm notices` and `pnpm notices:check`.
 */

/** Phrases a licence text must contain, and must not contain, per licence. */
const MARKERS = {
  "Apache-2.0": {
    has: ["Apache License", "Version 2.0"],
    not: ["GNU GENERAL PUBLIC LICENSE"],
  },
};

/**
 * Why `text` is not a text of `license`, or `null` when it matches (or no
 * markers are known for the licence).
 *
 * @param {string} license
 * @param {string} text
 * @returns {string | null}
 */
export function wrongText(license, text) {
  const markers = MARKERS[license];
  if (!markers) return null;
  const missing = markers.has.filter((phrase) => !text.includes(phrase));
  const unexpected = markers.not.filter((phrase) => text.includes(phrase));
  if (missing.length === 0 && unexpected.length === 0) return null;
  return (
    `the "${license}" licence text is wrong (missing: ${missing.join(", ") || "-"}; ` +
    `unexpected: ${unexpected.join(", ") || "-"})`
  );
}
