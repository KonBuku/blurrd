// Conventional Commits rules for blurrd, see CONTRIBUTING.md.
// Checked locally by the lefthook commit-msg hook and in CI on pull request
// titles.

/** @type {import("@commitlint/types").UserConfig} */
export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    "header-max-length": [2, "always", 72],
    "body-max-line-length": [2, "always", 72],
    "scope-enum": [
      2,
      "always",
      [
        "core",
        "blur",
        "video",
        "detect",
        "ocr",
        "engine",
        "cli",
        "app",
        "ui",
        "testkit",
        "xtask",
        "docs",
        "ci",
        "deps",
        "release",
        "repo",
      ],
    ],
  },
};
