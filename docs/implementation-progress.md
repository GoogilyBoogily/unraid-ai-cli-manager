# Implementation progress

Plan: `docs/superpowers/plans/2026-11-11-ai-cli-manager.md`

The user approved native execution, the exact eight-crate proposal, its runtime-tool amendment, and one independent final review on 2026-11-11. Production changes remain prohibited during desktop development.

Pre-flight: Tasks 1–10 share Environment, domain types, and private CLI/job contracts. Tasks 3–5 share committed-generation validation. Tasks 6–8 consume the same operation dispatcher. No conflicting interface names found.

Ruling: Use a feature branch in the existing empty repository rather than duplicate its only files into a worktree. There is no existing product implementation to isolate. Cost if wrong: planning edits share the checkout, while main remains protected by the feature branch.

Task 1: in progress. Cargo manifest pins the approved versions. Tests are written before the CLI implementation.

Evidence: CLI help was RED, then GREEN; CLI/fixture tests 4/4 passed. Shared model warnings remain until their callers land. Full Task 1 contract isn't marked complete yet.

Ruling: Serena editing is unavailable because this project has no language server configured. Use normal patch tools rather than install another unapproved dependency.

User authorization: Unsigned task commits are permitted for this repository after the signing key could not unlock. Use a per-command Git override. Don't modify signing configuration.

Task 2 evidence: The detached-writer and busy-job tests were RED before implementation. Both now pass. The timeout trace showed zombie descendants because rustix waitpid(None) only reaps the current process group. Switching to wait() reaps adopted children across groups; the original test now passes unchanged. Also reserve time for KILL inside short injected deadlines.

Ruling: Common lock parents such as /var/run must keep their existing permissions. Only manager-owned directories get private_directory permissions.
