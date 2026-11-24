# Migration track notes

Each parallel migration track writes `migration-notes/<track>.md` on its own branch instead of editing `RUST_MIGRATION.md` directly. During integration the notes are folded into `RUST_MIGRATION.md` and the note file is removed.

A note covers:

- What the track implemented, in the style of the "Current implementation" section.
- Intentional differences from the legacy command, one table row each: invocation, legacy behavior, Rust behavior. Each needs a test marked `xfail(LEGACY, strict=True)` or an explanation of why it cannot have one.
- Anything not done, not verified, or needing a decision.
