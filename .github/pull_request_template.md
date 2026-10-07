## What and why

<!-- What does this change and why? Link issues, for example "Closes #123". -->

## How tested

<!-- Commands you ran, the clips you used (your own recordings, never shared
here) and before/after numbers for performance changes. -->

## Checklist

- [ ] The title follows Conventional Commits (`feat(detect): ...`); it becomes the squashed commit header
- [ ] `pnpm fmt:check`, `pnpm lint`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo nextest run --workspace` pass
- [ ] Tests are added or updated for every change in behaviour
- [ ] Documentation (README, `docs/`) is updated if behaviour, flags or settings changed
- [ ] No footage, frames, screenshots or OCR output with readable player names is added
- [ ] Adapted third-party code carries a header and the notices are updated
