# Implementation progress

Plan: `docs/superpowers/plans/2026-11-11-ai-cli-manager.md`

The user approved native execution, the exact eight-crate proposal, its runtime-tool amendment, and one independent final review on 2026-11-11. Production changes remain prohibited during desktop development.

Pre-flight: Tasks 1–10 share Environment, domain types, and private CLI/job contracts. Tasks 3–5 share committed-generation validation. Tasks 6–8 consume the same operation dispatcher. No conflicting interface names found.

Ruling: Use a feature branch in the existing empty repository rather than duplicate its only files into a worktree. There is no existing product implementation to isolate. Cost if wrong: planning edits share the checkout, while main remains protected by the feature branch.

Task 1: implemented. Cargo manifest pins the approved versions. CLI/domain/fixture contracts have passing behavior tests.

Evidence: CLI help was RED, then GREEN; CLI/fixture tests 4/4 passed. Shared model warnings remain until their callers land. Full Task 1 contract isn't marked complete yet.

Ruling: Serena editing is unavailable because this project has no language server configured. Use normal patch tools rather than install another unapproved dependency.

User authorization: Unsigned task commits are permitted for this repository after the signing key could not unlock. Use a per-command Git override. Don't modify signing configuration.

Task 2 evidence: The detached-writer and busy-job tests were RED before implementation. Both now pass. The timeout trace showed zombie descendants because rustix waitpid(None) only reaps the current process group. Switching to wait() reaps adopted children across groups; the original test now passes unchanged. Also reserve time for KILL inside short injected deadlines.

Ruling: Common lock parents such as /var/run must keep their existing permissions. Only manager-owned directories get private_directory permissions.

2026-11-12 resume: Full current suite passes 16 tests. Snapshot capture/publication, retention, pinned filesystem handles, sensitive permissions, SQLite/WAL backup, cold restore, and required external-link failures are implemented. Remaining plan edge cases and full lint remain open, so the plan tasks aren't falsely marked complete.

The checked storage lease now pins the tool root as well as appdata. Atomic success receipts use the already checked directory, avoiding a fresh rootfs pathname lookup. Job states and installation/persistence outcomes are typed enums. Malformed/crashed worker responses become explicit failed jobs.

Build tooling: sudo required a password. The approved PHP/musl packages were downloaded and extracted only under /tmp/ai-cli-manager-build-tools. PHP CLI runs there. Rust's musl target is installed. No production tool or data was touched.

Ruling: Keep the shipped approved-vendor catalog empty until actual server validation exists. Synthetic fixture catalogs exercise installs/updates. Cost: production fresh installs and manager updates need a reviewed target added after server review; existing valid installations can still be adopted and persisted.

2026-11-12 continuation: Migration integrates reviewed fingerprints, independent per-tool schedule takeover, appdata policy before-images, rollback CAS, and interrupted-journal recovery. Tests cover both schedule copies, cron, unknown scripts, rollback after cron failure, multiple owned cron lines, and concurrent unrelated edits.

Lifecycle/status are integrated. A detached writer is cancelled and reaped before final backup, admission rejects work while stopping, stale update ticks obey Automatic updates OFF, and the post-hook filters this plugin after registration. Status exposes actual versions/receipts and flags unapproved versions. Native update suppression is explicitly unverified until server gates pass.

The native settings page and strict PHP request boundary are present. Rust tests call the pure PHP validator for authentication, CSRF, field types, fixed action arguments, and injection rejection. Static package automation, hooks, draft-only workflows, and operator docs are now present. Remaining work: package integration, additional failure cases, full checks, the one approved independent review, and GitHub push.

Ruling: The GitHub Workflows skill's referenced ci_monitor.cjs is absent from both the skill and installed roots. Use live official workflow documentation and exact action revisions from upstream Git refs, then validate observable CI through available GitHub tools. Don't stop implementation to install a missing wrapper.

Capacity fixture correction: a 1 PiB sparse test file exceeded the desktop filesystem's file-size limit before exercising manager behavior. The fixture now derives its sparse length from statvfs available bytes plus the reserve. The original capacity-rejection and previous-generation assertions remain unchanged.


Desktop implementation is complete for server review. Tasks 1–10 are represented by the shipped CLI/domain, jobs/process control, generations/SQLite/tool state, update/configuration, migration, Unraid lifecycle, PHP/UI, packaging/workflows/docs, and integrated fixture acceptance. The plan's original checkbox list remains the detailed design record rather than a claim of exhaustive fault-instruction coverage. See desktop-validation.md for the executed tests and limits.

Final local acceptance: 51 tests passed with test-support and artifact tests included. Production-feature run passed 10 tests, with its artifact test already covered by the first run. Both Clippy configurations and formatting/PHP/JS/shell checks passed. Static package build and artifact/ELF/license/fixture-guard inspection passed.

One independent review found five issues: database discovery/WAL race, incomplete-migration scheduling, Codex installer metadata overriding suppression, false version-policy verification, and relative external auth links. Each has a passing regression after correction. Package dependency notices and 0755 root were added. Uninstall supervisor completion and recovery cron refresh also have regressions. The independent review wasn't repeated, per the user's one-review choice.

No actual Unraid validation, beta publication, CA listing, or production migration is claimed. The empty approved catalog remains a release gate, not unfinished desktop installer plumbing. GitHub push and CI result are recorded below when available.
