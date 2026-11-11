# Rust dependency shortlist

**Prepared:** 2026-11-11
**Status:** Proposal for approval. No manifest, lockfile, dependencies, or product code have been added.
**Decision baseline:** [Interview decisions](decisions/unraid-ai-cli-manager-decisions.md)

I recommend eight direct crates, one synchronous executable, and the existing Unraid PHP integration. Keep rsync for file transfers and curl for downloads. Rust owns validation, configuration edits, locking, deadlines, and SQLite backups.

## Proposed direct crates

Versions below were checked against maintainer-published documentation on 2026-11-11. They're proposed starting pins, not a claim that the dependency graph has been compiled or audited.

| Crate | Proposed version | Features | Job |
| --- | --- | --- | --- |
| [serde](https://docs.rs/crate/serde/1.0.229) | `=1.0.229` | Defaults plus `derive` | Typed manager settings, generation manifests, job records, and status responses. |
| [serde_json](https://docs.rs/crate/serde_json/1.0.154) | `=1.0.154` | Defaults | JSON encoding, Claude configuration merges, and surgical UserScripts schedule edits. |
| [clap](https://docs.rs/crate/clap/4.6.7) | `=4.6.7` | Defaults plus `derive` | Typed command/action/tool parsing, consistent validation, and help. |
| [thiserror](https://docs.rs/crate/thiserror/2.0.21) | `=2.0.21` | Defaults | Explicit domain errors with operation context and preserved causes. |
| [toml_edit](https://docs.rs/crate/toml_edit/0.25.19+spec-1.1.0) | `=0.25.19` | Defaults: `parse`, `display` | Update owned Codex policy keys while preserving comments and unrelated configuration. |
| [rustix](https://docs.rs/crate/rustix/1.1.6) | `=1.1.6` | Defaults plus `fs`, `process` | Legacy-compatible advisory locks, capacity checks, ownership, process signals, subreaping, and stable process handles. |
| [rusqlite](https://docs.rs/crate/rusqlite/0.42.0) | `=0.42.0` | Defaults OFF, `backup`, `bundled` ON | SQLite online backup steps, bounded busy handling, and `PRAGMA quick_check`. |
| [tempfile](https://docs.rs/crate/tempfile/3.27.0) | `=3.27.0` | Defaults | Private installer/staging files, atomic file replacement, and isolated test roots. |

`toml_edit` publishes build metadata as `0.25.19+spec-1.1.0`. Cargo version requirements don't select by build metadata. Pin `=0.25.19` and record the resolved release in `Cargo.lock`.

These are runtime dependencies also reused by tests. No extra test framework is proposed. Rust's test harness and a small Rust fixture-command executable cover integration tests. Exact transitive versions and enabled features need review after the first approved lockfile is resolved. Don't silently substitute another direct crate or version if resolution fails.

## SQLite choice

Use a private, statically linked SQLite build through `rusqlite`. Its [release manifest](https://raw.githubusercontent.com/rusqlite/rusqlite/v0.42.0/Cargo.toml) supports `backup` and `bundled` and declares Rust 1.89 as its minimum. The bundled path uses a C compiler through [`libsqlite3-sys`](https://raw.githubusercontent.com/rusqlite/rusqlite/v0.42.0/libsqlite3-sys/Cargo.toml). It doesn't install or replace Unraid's SQLite library.

Call [`Backup::step`](https://docs.rs/rusqlite/latest/rusqlite/backup/struct.Backup.html) in a deadline-aware loop. Both `Busy` and `Locked` need an explicit retry limit. Don't use the convenience completion loop as an unbounded substitute for the shutdown deadline. Validate the destination after the backup handle has finished.

The [SQLite online backup API](https://www.sqlite.org/backup.html) provides an individual database snapshot. It doesn't give one transaction across every database, configuration file, and history file. Source connections must not create missing databases, run migrations, or checkpoint the application's databases as a side effect of backup.

Why this crate earns its place: we need real SQLite busy/corruption tests and cancellable incremental backups. Driving a shell `.backup` command makes path quoting, progress, and retry ownership harder to inspect.

## Process and shutdown choice

Use `std::process::Command` with argument vectors and [`CommandExt::process_group`](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html#tymethod.process_group). The stable API can establish a group without a custom unsafe pre-exec closure.

A process group alone doesn't contain a descendant that creates a new session. Each job therefore needs a small Rust supervisor using [subreaping](https://docs.rs/rustix/latest/rustix/process/fn.set_child_subreaper.html), tracked child identities, and [process-handle signals](https://docs.rs/rustix/latest/rustix/process/fn.pidfd_send_signal.html). Reap adopted descendants and verify termination. A successful signal call isn't proof that storage writers have exited.

This is a proposed implementation mechanism, not a proven Unraid guarantee. Test children that double-fork, change process groups, ignore TERM, and keep a file open. A kernel task stuck in uninterruptible I/O can outlast a userspace cancellation deadline. Record that as a failure of the shutdown guarantee, rather than reporting a clean drain. The plan must establish this boundary before enabling storage-writing jobs.

## Build target

I recommend `x86_64-unknown-linux-musl` for the release artifact and the native GNU target for desktop tests. A static release executable avoids accidentally requiring the desktop's glibc or a shared SQLite library on Unraid. Rust lists both targets in its [platform-support table](https://doc.rust-lang.org/rustc/platform-support.html), with different support tiers. Static versus dynamic C-runtime linkage still needs an [artifact check](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes).

- Start with the desktop's existing Rust 1.96.0 toolchain, edition 2024. Pin it in the planned `rust-toolchain.toml`, including rustfmt and clippy.
- Add the musl standard-library target and a musl C compiler for release builds. The SQLite C source must use that compiler too.
- Build generic x86-64 code. Don't use `target-cpu=native`.
- Reject a release executable with an ELF interpreter or unexpected shared-library dependencies. Also check architecture and the absence of test-root overrides.
- Run the artifact on actual Unraid 7.3.4 before marking the target supported. Static linking doesn't prove kernel/filesystem compatibility.

Observed locally: Rust/Cargo 1.96.0 and the GNU target are present. rsync, curl, tar, xz, and clang are on PATH. The musl target, `musl-gcc`, PHP, and the sqlite3 CLI weren't found by the checks run for this proposal. No toolchain components or OS packages were installed.

## Proposed runtime dependency amendment

The handoff lists rsync, jq, sqlite3, flock, and timeout as required tools. Rust makes four of those checks redundant. Approval of this proposal includes replacing that list as follows:

| Keep or verify on Unraid | Purpose |
| --- | --- |
| `rsync` | Full file-tree transfers with fixed exclusion lists. |
| `curl` | Official installer downloads with HTTP checks, redirects, and bounded timeouts. |
| `mountpoint` | Required `/mnt/user` mount guard before any share-path creation. |
| `bash`, `sh` | Vendor installer interpreters and minimal package/event entry points. |
| `logger`, native `notify`, `update_cron` | Unraid logging, notifications, and cron reconciliation. |
| Plugin Manager / Slackware package tools | Manager package lifecycle, not CLI vendor installation. |
| Unraid's existing PHP/web runtime | Settings page, authentication, CSRF validation, and fixed backend invocation. |

Don't require `jq`, `sqlite3`, `flock`, or `timeout` executables for manager operation. JSON, SQLite, advisory locking, and timeouts move into the Rust backend. Still interoperate with the legacy scripts' flock locks. Don't remove or replace any installed system tool or library.

Build/test tools are separate: Cargo/rustc/rustfmt/clippy, a native C compiler and archiver, a musl compiler plus Rust target for release builds, PHP CLI for request tests, and tar/xz/sha256sum/readelf for package validation. If desktop PHP or musl tooling needs installation, request that concrete installation separately before running it. CI may provision the approved build/test tools in its ephemeral runner.

## Keep out of v1

- No async runtime, Rust HTTP server, web framework, or always-running manager daemon. Each accepted operation gets a tracked worker/supervisor.
- No Rust HTTP/TLS stack. Use the existing curl command with a fixed executable and argument vector.
- No ORM, manager database, plugin system, dependency-injection framework, or generic filesystem abstraction.
- No custom cryptography. Plugin Manager verifies package SHA256. Build automation uses sha256sum.
- No hard-link snapshot reuse. Full copies simplify immutability and capacity accounting. Revisit only after representative measurements justify it.
- No new crates for timestamps, identifiers, directory walking, CLI testing, or benchmarks. Use the standard library and the approved crates.

## Approval scope

Review the eight direct crates and their starting versions/features, private bundled SQLite, the static-musl release target, and the runtime dependency amendment together. They're proposals, not additional user decisions already recorded as accepted.

Approval permits preparing the Cargo manifest and resolving its lockfile during implementation. It doesn't prove dependency security, target compatibility, database-format compatibility, or Codex updater suppression. Those need the checks in the [implementation plan](../superpowers/plans/2026-11-11-ai-cli-manager.md).
