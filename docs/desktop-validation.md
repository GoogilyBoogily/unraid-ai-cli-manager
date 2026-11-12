# Desktop validation

Validated on 2026-11-12. This evidence covers desktop fixtures and package inspection only. Production wasn't installed, migrated, rescheduled, or used as a test target.

## Environment and results

Rust and Cargo: 1.96.0. PHP CLI: 8.3.6. Release target: x86_64-unknown-linux-musl. PHP/musl packages were extracted under a private `/tmp` build-tools directory because system package installation required an unavailable sudo password. No system libraries were replaced.

| Check | Actual result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --locked --all-targets --features test-support -- -D warnings` | Passed, no warnings |
| `cargo clippy --locked --all-targets --no-default-features -- -D warnings` | Passed, no warnings |
| `cargo test --locked --features test-support -- --include-ignored` | 51 passed, 0 failed, 0 ignored |
| `cargo test --locked --no-default-features` | 10 passed, 0 failed, 1 artifact test ignored in this invocation. That test passed in the fixture-feature run above |
| PHP syntax, both request files | No syntax errors |
| `node --check unraid/scripts/settings.js` | Passed |
| `bash -n packaging/build.sh` and `sh -n` on all hooks | Passed |
| Static package build | Passed, approximately 1.6 MB `.txz` with SHA256 manifest |
| ELF inspection | ELF64 x86_64, no interpreter or shared-library dependency |
| Artifact inspection | Root-owned files, staging root 0755, no vendor packages or CLI state, dependency notices present, release rejects fixture roots |
| `git diff --check` | Passed |

Local PHP test commands used `AI_CLI_MANAGER_PHP=/tmp/ai-cli-manager-build-tools/root/usr/bin/php8.3`. The package build used `CC_x86_64_unknown_linux_musl` and `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER` pointing to `/tmp/ai-cli-manager-build-tools/musl-gcc`. Normal CI uses runner-installed PHP and musl-tools.

## Coverage

| Area | Evidence |
| --- | --- |
| Storage boundaries | Missing mounts never create persistence paths. Pinned-handle test renames the mounted-path fixture and proves writes don't redirect to replacement rootfs paths |
| Persistence | Missing marker/source/dependency/package, partial transfers, disk-full exit, capacity preflight, unchanged failed-backup receipt/selector, current/previous retention, private permissions |
| SQLite | Live WAL snapshot, quoted filenames, corruption, bounded busy deadline, deleted databases, and a database created during rsync with an open WAL writer |
| State and links | Complete Codex package/helpers, missing/escaping helper, external absolute/relative auth links preserved without copying targets, missing required target, runtime socket link exclusion |
| Updates | Automatic OFF with Update Now, identical/newer manual versions, failed candidate rollback, install success with persistence failure, fresh Codex opt-in, updater metadata exclusion and daemon package preservation |
| Migration | Claude and Codex takeover, both schedule copies, retained legacy state, independent jobs, unknown scripts, existing legacy lock, cron failure, concurrent unrelated edits, multiple cron lines, interrupted-journal recovery and schedule gating |
| Lifecycle | Detached writer cancellation/reaping, same-tool Busy, stale cron OFF, post-registration cron filtering, bounded stop/final backup, uninstall preservation, notification deduplication/reset |
| Web boundary | Auth/CSRF rejection, strict action/field/type allowlists, fixed arguments, consent and settings stdin, command/path injection rejection |
| Integration | Inert initial startup, explicit adoption, state change/backup, offline RAM loss/restore, disable preserving persisted state |

Tests use synthetic installations. They don't execute the supplied legacy persistence scripts or official vendor installers against a real CLI home. Fault fixtures exercise selected publication/migration interruption boundaries rather than proving every possible crash instruction.

## Independent review

One independent read-only review was performed, as authorized. It found five actionable defects. All five now have focused passing regressions:

- New-database/WAL discovery race: filename exclusions, stable source/staged database inventories, failed publication on changes.
- Interrupted migration scheduling: jobs, cron, and readiness gate nonterminal journals. Recovery refreshes cron before returning.
- Codex updater metadata overwrite: candidate copies exclude selection metadata and the daemon tree.
- False verified-policy claim for a newer manual version: verification now requires the installed version to match the approved target.
- Relative external auth links: validation resolves the intended live location without copying the external target.

The review also identified missing dependency notices and private staging-root archive permissions. The package now retains exact resolved dependency notices, SQLite's upstream public-domain notice, and a 0755 staging root. A separately discovered completed-supervisor/uninstall race now has a regression proving supervisors finish before cron refresh.

The reviewer didn't independently re-review subsequent fixes. The final command results above were run after those fixes. Actual native UI rendering, Unraid event/cron semantics, production-sized timings, and vendor updater behavior remain in [server-validation.md](server-validation.md). No public release or Community Applications submission was performed.
