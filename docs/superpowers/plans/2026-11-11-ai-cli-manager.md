# Unraid AI CLI Manager Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Use superpowers:subagent-driven-development only if the user explicitly chooses delegation. Steps use checkbox syntax for tracking.

**Goal:** Build the desktop-tested Rust backend, native Unraid settings page, and reviewable plugin package that replace the two legacy persistence UserScripts without changing production during development.

**Architecture:** One synchronous Rust executable runs public CLI requests, short internal Unraid actions, and tracked background jobs. Persistence/tool logic is separate from Unraid schedule, notification, and web integration. PHP validates requests and calls fixed backend actions through argument arrays.

**Tech Stack:** Rust 1.96.0 / edition 2024, Rust tests, PHP, existing rsync/curl, Slackware `.txz`, and Unraid `.plg`. The [eight-crate shortlist](../../context/rust-dependency-shortlist.md), private bundled SQLite, static-musl release target, and revised runtime-tool requirements were approved on 2026-11-11.

**Execution evidence:** [Desktop validation](../../desktop-validation.md) and [implementation ledger](../../implementation-progress.md). Original checklist retained for traceability. Actual server gates remain pending.

**Spec:** [Interview decisions and handoff constraints](../../context/decisions/unraid-ai-cli-manager-decisions.md). Read that record and the dependency proposal before executing this plan.

## Global constraints

- Desktop development must not install on production, change its schedules, migrate its data, or copy its private CLI state.
- Root's host-native installations only. No Docker execution, embedded terminal, or multi-user support in v1.
- Never create paths beneath `/mnt/user` until `mountpoint -q /mnt/user` succeeds.
- Manager settings live under `/boot/config/plugins/ai-cli-manager`. Credentials, histories, snapshots, and vendor packages stay off flash.
- Preserve `/mnt/user/appdata/claude-code` and `/mnt/user/appdata/codex` as persistence roots. Live state stays in RAM.
- Reuse `/var/run/claude-persist.lock` and `/var/run/codex-persist.lock` during and after migration.
- Automatic update checks run daily at 04:00 server time. Per-tool Automatic updates defaults ON. Update Now works with it ON or OFF.
- Backups default to every 15 minutes. Accepted settings are exactly 5, 15, 30, and 60 minutes.
- Installation alone leaves both tools unmanaged. Initialize/Adopt enables each tool only after success. Disabled tools get no automatic restore, backup, or update.
- Migration and rollback are independent per tool. Preserve unrelated schedules and never silently reactivate legacy jobs on disable or uninstall.
- Publish only validated, immutable generations. Retain current and previous. Automatic cleanup is limited to older manager-created generations and abandoned staging. Preserve legacy migration backup.
- A missing live source is an error. A partial copy, database failure, or failed publication cannot advance last-successful-backup status.
- Use umask 077, sensitive root directories as 0700, and known sensitive files as 0600 while preserving executable permissions.
- Restore only committed generations. Conflicting live state without valid initialization requires Needs attention. Don't overwrite active sessions.
- Each SQLite database is individually consistent. Online backup isn't one transaction across all files.
- Approved target versions come with the manager release. Untested manual versions get Needs attention but may continue validated persistence. Never automatically downgrade.
- Keep vendor manual update commands available. Disable native unattended updaters for managed tools. Don't automatically update/restart a running Codex daemon or terminate user CLI sessions.
- Fresh Codex installation needs explicit opt-in before changing to unsandboxed host execution with on-request approvals. Don't change existing execution policy as a migration side effect.
- `stopping_svcs` has a 120-second total budget, including worker cancellation and final-backup attempts. Verify storage-writing descendants have exited before treating shutdown as drained.
- Test minimum is Unraid 7.3.4. Exclude Unraid 8, including prereleases. Desktop mocks cannot establish actual Unraid support.
- License manager code under MIT. Don't bundle vendor CLIs or imply endorsement. No AI/tool attribution in commits or PR descriptions.
- Don't add dependencies until the exact shortlist is approved. Don't add new public CLI verbs in implementation without approval.

## Review focus

These are the five most consequential cases that a happy-path build would miss. Their tests are assigned below.

1. Storage vanishes after a successful initial mount check. A stop race must not recreate `/mnt/user` on rootfs or publish a partial generation. Task 3 owns this case.
2. A worker double-forks, calls setsid, ignores TERM, or leaves a writer behind. Cancellation must track/reap owned descendants without signalling a recycled PID or a user CLI session. Task 2 owns this case.
3. UserScripts edits an unrelated schedule between takeover and rollback. Rollback must preserve that edit and detect conflicts in the manager's own entries. Task 6 owns this case.
4. A manual vendor update installs a newer, unapproved version or changes the launcher. Valid backups continue, automatic downgrade is refused, and missing helpers still fail validation. Task 5 owns this case.
5. A supported-looking request reaches the PHP handler with no authenticated session, a missing/invalid CSRF token, or shell syntax in a field. Reject it before starting any backend job. Task 8 owns this case.

## Proposed implementation calls

These are builder recommendations for review, not extra user decisions already accepted.

- Use a binary crate with private modules, not a framework or generic service layer. Keep production root paths fixed. Compile fixture-root/program overrides only behind `test-support`.
- Use full snapshot copies in v1. Don't use hard links, reflinks, or rsync in-place updates against committed data.
- Hold a checked appdata-directory handle only for the duration of each storage job. Use directory-relative operations or paths anchored to that handle, not a fresh pathname lookup that can land on rootfs after unmount. This concrete storage lease must verify the mounted filesystem identity after the initial mountpoint check. It isn't a generic filesystem abstraction.
- `current` is a relative symlink into `generations/`. Each immutable manifest records its predecessor. No additional previous-selector transaction is needed.
- Put ordinary jobs and transient status under `/run/ai-cli-manager`, with root-only permissions. Keep small migration/settings journals on flash because takeover must survive reboot. Keep backup-success receipts beside snapshots in appdata.
- Public operations return tracked job receipts. `--tool all` submits separate tool jobs and preserves separate results. Don't add a permanent daemon or a queue of repeated missed cron jobs.
- A disabled tool rejects automated work. Explicit maintenance commands may still be requested, but don't enable scheduling or change native-updater suppression as a side effect. Initialize remains the only first-time takeover path.
- Use the public interface `ai-cli-manager status|restore|backup|install|update --tool claude|codex|all`, plus `--json` for structured output. Private `__internal` actions handle initialization, settings, worker supervision, events, and plugin hooks. They aren't supported public CLI verbs and accept only fixed enums/bounded JSON input.
- Proposed regular-operation deadline: 600 seconds total. Installer download uses a 10-second connect timeout and 60-second transfer timeout inside that budget. Bound SQLite busy retries to 20 waits of 100 ms, and check the overall deadline between page steps.
- Shutdown uses one absolute 120-second deadline: up to 20 seconds to cancel/drain ordinary work, final backups only until second 100, and the last 20 seconds for cancellation/reaping. Enabled tools may attempt final backup concurrently under their separate locks. Don't start a final backup while its previous worker still writes.
- Keep old vendor packages in live RAM during v1. Snapshot retention must not become live-package garbage collection.

## File map

Create files only when their task needs them. The directories below are a responsibility map, not permission to scaffold unused code.

| Files | Responsibility |
| --- | --- |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.gitignore` | Approved dependencies, reproducible builds, test-only feature, and build-output ignores. |
| `src/main.rs`, `src/domain.rs`, `src/environment.rs` | Argument dispatch, domain types/errors, fixed production roots, and fixture boundaries. |
| `src/process.rs`, `src/jobs.rs` | Bounded subprocesses, supervisor/child identities, admission control, job records, and legacy locks. |
| `src/generations.rs`, `src/sqlite.rs` | Staging/publication/retention and individually consistent database backups. |
| `src/tools/mod.rs`, `src/tools/claude.rs`, `src/tools/codex.rs` | Required files, exclusions, package layout, link validation, merges, and restore mapping. |
| `src/configuration.rs`, `src/operations.rs` | Manager/vendor settings, initialization, install/update/restore orchestration, and status synthesis. |
| `src/migration.rs`, `src/unraid.rs` | Per-tool UserScripts journal/rollback, events, cron, notifications, and uninstall. |
| `src/bin/fixture-command.rs`, `tests/common/mod.rs`, `tests/fixtures/` | Test-only command/process helper and synthetic filesystem/vendor/Unraid fixtures. |
| `tests/cli.rs`, `tests/processes.rs`, `tests/persistence.rs`, `tests/tool_state.rs`, `tests/updates.rs`, `tests/migration.rs`, `tests/lifecycle.rs`, `tests/web_requests.rs`, `tests/packaging.rs`, `tests/end_to_end.rs` | Behavior-level tests through the fixture executable/environment. |
| `unraid/CommandLineTools.page`, `unraid/include/actions.php`, `unraid/include/request.php`, `unraid/scripts/settings.js` | Native settings view, validation, fixed backend calls, and job polling. |
| `unraid/event/disks_mounted`, `unraid/event/stopping`, `unraid/event/stopping_svcs`, `unraid/plugin-post-hook` | Minimal executable entry points into private Rust actions. |
| `packaging/ai-cli-manager.plg.in`, `packaging/build.sh`, `packaging/approved-versions.json`, `assets/ai-cli-manager.svg` | Package template/build, shipped target metadata, and original icon. |
| `.github/workflows/ci.yml`, `.github/workflows/release.yml` | Desktop verification, package artifacts, and manually invoked draft releases. |
| `README.md`, `LICENSE`, `CHANGELOG.md`, `docs/migration.md`, `docs/recovery.md`, `docs/server-validation.md`, `docs/upstream-contracts.md` | Installation/authentication instructions, recovery/export, provenance, and release evidence. |

## Shared contracts

Define these once in Task 1. Later tasks consume them rather than inventing parallel models. All Rust interfaces below are crate-private unless they're test-only helpers.

- `Tool`: `Claude | Codex`. `ToolSelection`: `Claude | Codex | All`.
- `Operation`: `Initialize | Restore | Backup | Install | Update`.
- `ManagerError`: explicit storage, dependency, busy, initialization, validation, capacity, command, database, deadline, migration-conflict, and publication failures. Preserve causes internally. Expose allowlisted error codes/messages rather than configuration contents or raw subprocess output.
- `Result<T>`: `std::result::Result<T, ManagerError>`.
- `Environment`: validated paths, fixed/injected program paths, and production/test mode. Only its checked constructors create it.
- `JobRequest { tool, operation }`, `JobReceipt { job_identifier }`, `JobRecord`: queued/running/succeeded/failed/cancelled plus stage, worker identity, and separate installation/persistence outcomes.
- `StatusReport`: `schema_version: 1`, per-tool readiness/operation state, restart flag, installed/cached/daemon versions, last successful backup/update, and safe errors. Preserve simultaneous conditions rather than flattening an unsupported-version warning and a failed backup into one boolean.
- JSON status uses `tools.claude` and `tools.codex`, each with `readiness`, `operation`, `restart_required`, `versions`, `last_successful_backup`, `last_successful_update`, and `errors`. A waiting tool reports `readiness: "waiting_for_storage"`. Submission conflicts return code `busy`. Job outcomes use separate `installation` and `persistence` values such as `succeeded`, `failed`, and `not_attempted`.
- Timeouts use `std::time::Instant` deadlines. Tests may shorten injected deadlines. Production constants remain the exact values above.
- `GenerationManifest`: format version 1, tool, generation identifier, predecessor identifier, creation time, validated package selection/version, and required state/database paths. Restore destinations come from the tool adapter, never arbitrary manifest paths.

### Task 1: CLI, safe fixture roots, and source contracts

**Files:** Create the Cargo/toolchain files, `src/main.rs`, `src/domain.rs`, `src/environment.rs`, `tests/common/mod.rs`, `tests/cli.rs`, `tests/fixtures/legacy/`, and `docs/upstream-contracts.md`.

**Interfaces:** Produce `Environment::production() -> Result<Environment>`, test-only `Environment::fixture(root: &Path, programs: Programs) -> Result<Environment>`, and `read_status(environment: &Environment, selection: ToolSelection) -> Result<StatusReport>`. `Programs` is a concrete struct of named executable paths, not a generic command injection interface.

- [ ] Obtain approval for the exact dependency proposal and proposed implementation calls. Resolve and commit the lockfile only afterward. If resolution changes a direct version or requires another direct crate, return the concrete difference for approval.
- [ ] Preserve the two user-supplied scripts as nonexecuted behavioral-reference fixtures. Record hashes of those fixture bytes and their source. Don't claim pasted bytes are proven identical to production files.
- [ ] Record current official installer/version-control interfaces, UserScripts schedule fields/cron format, and Unraid hook/auth/CSRF conventions with source revisions in `docs/upstream-contracts.md`. Use upstream source and synthetic records only. If a format can't be verified, the corresponding migration implementation must fail closed until a verified fixture is available.
- [ ] Write `fixture_root_refuses_root_or_symlink_escape`, `release_cli_rejects_fixture_overrides`, `status_waits_without_creating_share_paths`, and `cli_rejects_unknown_action_tool_or_interval`. Assert rejected requests cause zero child-command invocations and no share-directory creation.

  ```rust
  assert_eq!(status["tools"]["claude"]["readiness"], "waiting_for_storage");
  assert!(!fixture_root.join("mnt/user/appdata").exists());
  ```

- [ ] Run the new tests and confirm the expected missing implementation fails. Implement the typed CLI, constructors, safe status skeleton, and root guards. Production must reject test-root variables/options even when launched outside Unraid.
- [ ] Run `cargo test --locked --features test-support --test cli`, `cargo fmt --check`, and `cargo clippy --locked --all-targets --features test-support -- -D warnings`. Expect all to pass, then commit only this task's files.

Deliverable: A safe status/CLI entry point and a fixture harness that cannot fall through to real host paths or programs.

### Task 2: Tracked jobs, shared locks, and complete cancellation

**Files:** Create `src/process.rs`, `src/jobs.rs`, `src/bin/fixture-command.rs`, and `tests/processes.rs`. Extend main dispatch for hidden worker actions.

**Interfaces:** Consume Task 1 models. Produce `submit_job(environment: &Environment, request: JobRequest) -> Result<JobReceipt>`, `read_job(environment: &Environment, identifier: &str) -> Result<JobRecord>`, `cancel_job(environment: &Environment, identifier: &str, deadline: Instant) -> Result<()>`, and `run_command(environment: &Environment, command: &CommandSpec, deadline: Instant) -> Result<CommandOutput>`. `CommandSpec` contains a fixed program choice and `OsString` argument vector. `CommandOutput` stays private to the operation.

- [ ] Write tests for two jobs on one tool returning busy, separate-tool jobs proceeding independently, legacy `flock` contention, and the stop-admission race. Acquire the admission lock before checking stopping state and registering accepted work. Never hold it during a long operation.
- [ ] Add fixture modes that fork, detach, ignore TERM, write a sentinel, and leave pipes open. Write `cancellation_reaps_detached_writers`, `recycled_process_identity_is_not_signalled`, and `timeout_reaps_before_return`. Assert the writer stops, no owned descendant remains, and an unrelated fixture process survives.

  ```rust
  assert_eq!(second_submission["code"], "busy");
  assert!(remaining_owned_descendants.is_empty());
  assert!(unrelated_process.try_wait()?.is_none());
  ```

- [ ] Run the failing tests. Implement a supervisor that stays outside the worker process group, enables child subreaping, tracks process identities/handles, and reaps adopted descendants. Commands use argv, fixed child environments, bounded output collection, and deadlines. Don't introduce unsafe signal handlers or pipe readers that can hang forever.
- [ ] Keep per-tool legacy locks open for the complete operation. Write queued/running/completion records atomically under the private RAM job directory. Return a receipt only after registration is acknowledged. Surface supervisor startup failure instead of returning a successful receipt.
- [ ] Run `cargo test --locked --features test-support --test processes` and the Task 1 CLI tests. Expect no leaked processes or open fixture writers, then commit.

Deliverable: A proven desktop process boundary before persistence can write to appdata. An unsupported process primitive fails dependency/readiness checks explicitly.

### Task 3: Immutable full-copy generations and retention

**Files:** Create `src/generations.rs` and `tests/persistence.rs`. Extend fixture-command for transfer/capacity/publication faults.

**Interfaces:** Produce `capture_generation(environment: &Environment, tool: Tool, deadline: Instant) -> Result<CommittedGeneration>`, `read_current_generation(environment: &Environment, tool: Tool) -> Result<CommittedGeneration>`, and `prune_generations(environment: &Environment, tool: Tool) -> Result<()>`. `CommittedGeneration` is constructible only through validated publication/read paths.

- [ ] Write tests for missing storage/source/marker, insufficient capacity, partial rsync, ENOSPC, failed validation, and interruptions before/after generation rename and selector replacement. Assert the previous good selector stays usable and last-success status never advances on failure.
- [ ] Write `storage_disappearing_does_not_recreate_share_mount` with a transfer running while stopping/storage identity changes. Assert cancellation completes before simulated unmount, and post-loss attempts never create `/mnt/user` in the fixture rootfs.
- [ ] Write a directory-rename race test against the checked storage lease. Replace the visible fixture mount path while a directory handle is held. Assert no manager paths appear under the replacement rootfs directory. All storage writes, including external transfer destinations and publication, must remain anchored to the checked filesystem or fail.
- [ ] Write retention tests: third successful publication leaves current and predecessor, failed staging is never restorable, committed files don't share inodes with later mutable staging, and legacy backup is untouched. Reject `current` escaping the generation root, unknown manifest formats, symlinked manager control directories, and reserved-name collisions during migration.

  ```rust
  assert_eq!(selector_after_failure, selector_before_failure);
  assert_eq!(status_after["tools"]["codex"]["last_successful_backup"],
             status_before["tools"]["codex"]["last_successful_backup"]);
  assert_eq!(retained_complete_generations.len(), 2);
  ```

- [ ] Run failing tests. Stage under the persistence root on the same filesystem. Use full rsync copies into a fresh directory, then adapter validation, file/directory sync, generation rename, and atomic relative-selector replacement. Never create a fallback share directory in `/tmp` or on rootfs.
- [ ] Publish a separate success receipt only after successful publication/sync. A crash between publication and the receipt may conservatively retain an older success time. On receipt/sync failure, report the uncertainty and preserve the previous good generation rather than advertising a successful backup.
- [ ] Check capacity conservatively for a full copy while retaining existing generations and legacy data. Recheck during the job and handle real ENOSPC. Prune only manager-owned, unreferenced generations under lock after success. Run `cargo test --locked --features test-support --test persistence`, then commit.

Deliverable: Publication and cleanup that cannot replace the last good snapshot with incomplete data.

### Task 4: Tool state, SQLite, package links, and restore

**Files:** Create `src/sqlite.rs`, `src/tools/mod.rs`, `src/tools/claude.rs`, `src/tools/codex.rs`, and `tests/tool_state.rs`. Extend generation capture with adapter/database validation.

**Interfaces:** Produce `backup_database(source: &Path, destination: &Path, deadline: Instant) -> Result<()>`, `validate_tool_state(environment: &Environment, tool: Tool, candidate: &Path) -> Result<ValidatedToolState>`, and `restore_generation(environment: &Environment, tool: Tool, generation: &CommittedGeneration, deadline: Instant) -> Result<()>`.

- [ ] Write Claude tests for recursive JSON merge with live values winning, history-line deduplication, versioned executable selection, and legacy exclusions: shell-snapshots, session-env, cache, downloads, and paste-cache. Invalid JSON must return a contextual error and leave the live file intact.
- [ ] Write Codex tests for complete package/helpers, live WAL writes during backup, Busy/Locked retry exhaustion, corruption, and a database deleted since the preceding generation. Include a database path with spaces/quotes. Assert successful destinations pass quick_check and deleted databases/sidecars aren't resurrected.

  ```rust
  assert_eq!(destination.query_row("PRAGMA quick_check", [],
             |row| row.get::<_, String>(0))?, "ok");
  assert!(!new_generation.join("root/.codex/deleted.sqlite").exists());
  ```

- [ ] Establish database discovery from the verified supported-release layouts. Exclude identified live database files and their WAL/SHM/journal siblings from rsync. Back up them only through SQLite. A corrupt recognized database is a failure, not a reason to copy it as an ordinary file. Prune package trees from database discovery.
- [ ] Write socket/PID/runtime-lock exclusions, external-symlink preservation, required missing-target failures, permissions, and package selectors escaping into live installation tests. Do not traverse external links or chmod their targets. Normalize only known package-owned selectors into snapshot-local links.
- [ ] Run failing tests. Keep a fixed restore map for each tool's managed paths. Generation payloads mirror only the managed root-relative paths under `root/`. Codex's complete package tree stays included. Validate candidate package execution from a private RAM staging location, never by following a cached selector into the live installation.
- [ ] Restore only from a validated generation after checking initialization and active processes. Unknown conflicting live state is Needs attention with no overwrite. RAM stays authoritative when the boot marker already exists. An interrupted initialization journal may permit safe, inactive recovery merges, but never authorize overwriting an active session. Set the boot marker only after every required restore step succeeds.
- [ ] Enforce 0700 directories/0600 known sensitive files while preserving executables. Run `cargo test --locked --features test-support --test tool_state` plus persistence tests. Expect source data unchanged by SQLite reads and no marker after failure, then commit.

Deliverable: Correct tool-specific snapshots and cold-boot restore, including real SQLite consistency tests.

### Task 5: Initialization, update ownership, and transactional installation

**Files:** Create `src/configuration.rs`, `src/operations.rs`, `packaging/approved-versions.json`, `tests/updates.rs`, and synthetic vendor fixtures.

**Interfaces:** Produce `initialize_tool(environment: &Environment, tool: Tool, consent: InitializationConsent, deadline: Instant) -> Result<OperationOutcome>`, `install_or_update(environment: &Environment, tool: Tool, operation: Operation, deadline: Instant) -> Result<OperationOutcome>`, and `save_settings(environment: &Environment, changes: SettingsChange) -> Result<()>`. `OperationOutcome` separates installation and persistence results. `InitializationConsent` explicitly records fresh Codex host-execution opt-in.

- [ ] Write tests that plugin installation is inert, Initialize adopts valid live state without restoring over sessions, and first-time activation commits enabled state only after initialization/takeover succeeds. An enabled checkbox alone must not perform first-time takeover.
- [ ] Test automatic updates ON/OFF, manual Update Now in both modes, same-version skip, missing approved target, older/newer/unparseable manual versions, and native manual command availability. Untested but structurally valid state still backs up/restores. Never automatically downgrade or guess an ordering for an unparseable version.
- [ ] Pin actual vendor target metadata only after reviewing official versioned installer/layout/update-policy evidence. Test data uses explicit fake versions. Don't label desktop-only results as proof of Unraid updater suppression.
- [ ] Write installer HTTP-error, timeout, noninteractive failure, missing helper, invalid version, selector rollback, and persistence-after-install failure tests. Assert the prior complete package/selection remains usable after failed validation and installation success is retained in the result when only persistence fails.

  ```rust
  assert_eq!(job["installation"], "succeeded");
  assert_eq!(job["persistence"], "failed");
  assert_eq!(selection_after_rejected_downgrade, manual_selection);
  ```

- [ ] Run failing tests. Download the official installer to a private file, check success, invoke its supported pinned-version interface, preserve vendor integrity checks, and validate the complete candidate before selecting it. Keep earlier packages. Use the Task 2 supervisor for all descendants and bounded waits.
- [ ] Apply only owned policy edits: Claude `DISABLE_AUTOUPDATER=1`, not `DISABLE_UPDATES`. Preserve unrelated JSON/TOML and record prior owned keys for failed initialization rollback. Codex policy paths/keys come from the supported-release evidence, including installer metadata and daemon controls. Missing evidence means unverified/Needs attention, not a false OFF claim.
- [ ] Preserve existing Codex execution settings. For a fresh installation, declining host-execution opt-in leaves vendor execution defaults intact. Only explicit opt-in may set unsandboxed host execution with on-request approvals. Show any platform limitation rather than bypassing it silently.
- [ ] Identify running CLI/daemon versions from verified executable identities where possible. Unknown versions remain explicitly unknown. Flag old-process policy/version as restart required without terminating it. Run `cargo test --locked --features test-support --test updates` and tool-state regression tests, then commit.

Deliverable: One install/update path shared by manual and scheduled triggers, with accurate failure reporting and policy ownership.

### Task 6: Independent migration with conflict-safe rollback

**Files:** Create `src/migration.rs`, `tests/migration.rs`, and synthetic schedules/cron fixtures based on verified upstream formats. Add small per-tool migration-journal support to configuration.

**Interfaces:** Produce `migrate_tool(environment: &Environment, tool: Tool, deadline: Instant) -> Result<MigrationOutcome>` and `recover_migration(environment: &Environment, tool: Tool) -> Result<MigrationOutcome>`. Each outcome identifies the tool and completed/rolled-back/needs-attention state.

- [ ] Write tests for expected/unknown script fingerprints, missing boot markers, existing legacy jobs, reserved-path collisions, and active CLI sessions. Main-script and wrapper verification must not execute the scripts. Unknown source bytes require review, not fuzzy matching.
- [ ] Write successful Claude-only and Codex-only takeover tests against both schedule copies and customSchedule.cron. Assert exactly that tool's three matching entries change and unrelated entries remain semantically identical. A failed Codex takeover must preserve a successful Claude takeover.
- [ ] Base fixtures on the [upstream schedule writer](https://raw.githubusercontent.com/Squidly271/user.scripts/master/source/user.scripts/usr/local/emhttp/plugins/user.scripts/exec.php): a JSON object keyed by full wrapper script path, with `script`, `frequency`, `id`, and `custom` fields. The [upstream page](https://raw.githubusercontent.com/Squidly271/user.scripts/master/source/user.scripts/usr/local/emhttp/plugins/user.scripts/Userscripts.page) uses `disabled`, `start`, `stop`, and `custom` frequency values. Change only the selected records' frequency to `disabled`, retain their other fields, and remove their matching custom cron lines. Verify the installed version matches this contract before server takeover.

  ```rust
  assert_eq!(updated_schedule[claude_backup_path]["frequency"], "disabled");
  assert_eq!(updated_schedule[codex_backup_path], original_schedule[codex_backup_path]);
  assert_eq!(updated_schedule[unrelated_path], concurrently_edited_unrelated_entry);
  ```

- [ ] Write crash recovery after each journal step, cron refresh failure, and `rollback_preserves_concurrent_unrelated_schedule_edit`. Rollback restores only owned fields/lines if their current values match the manager's after-image. Conflicting edits cause Needs attention rather than restoring an entire stale schedule file.
- [ ] Run failing tests. Journal before changes, acquire the legacy tool lock, wait for existing legacy work, validate the restored marker/live state, and build the first committed generation from RAM. Don't restore over active sessions. Detect queued legacy invocations before takeover completes.
- [ ] Preserve legacy state under legacy-migration-backup only after the live generation validates. Preserve original scripts and small manager/vendor setting preimages. Disable the selected tool's three jobs in both schedule files, remove only its generated cron entries, refresh cron, and enable its plugin scheduling only after takeover succeeds.
- [ ] Keep only schedule and manager-control metadata in the flash journal. Vendor configuration rollback copies belong in protected appdata, never on flash. Journal diagnostics must not serialize whole CLI configuration files.
- [ ] Make multi-file progress recoverable rather than pretending two JSON files and cron update atomically. An incomplete journal is reported and resolved before either scheduler is treated as the accepted owner. Run `cargo test --locked --features test-support --test migration`, then commit.

Deliverable: Per-tool adoption with a tested journal and no unrelated schedule loss.

### Task 7: Unraid lifecycle, cron, notifications, and status

**Files:** Create `src/unraid.rs`, the four minimal Unraid event/post-hook entry points, and `tests/lifecycle.rs`. Complete status synthesis in operations.

**Interfaces:** Produce `handle_event(environment: &Environment, event: UnraidEvent) -> Result<()>`, `reconcile_cron(environment: &Environment) -> Result<()>`, `handle_plugin_hook(environment: &Environment, arguments: &[OsString]) -> Result<()>`, and `uninstall_manager(environment: &Environment, deadline: Instant) -> Result<()>`. `UnraidEvent` is the fixed disks_mounted/stopping/stopping_svcs enum.

- [ ] Write tests for hot install before/after plugin registration, repeated array start, settings changes, disabled tools, and unchanged cron. Verify 04:00 daily update entries and 5/15/30/60 backup schedules. Cron writes occur only when content changes.

  ```rust
  assert!(cron.contains("0 4 * * *"));
  assert_eq!(accepted_backup_intervals, vec![5, 15, 30, 60]);
  assert!(remaining_storage_writers.is_empty());
  ```

- [ ] Test post-hook filtering: only successful install/update registration for ai-cli-manager.plg triggers reconciliation. Current upstream passes type, method, plugin basename, and an empty error on success. Confirm that contract against supported Unraid before release.
- [ ] Write stopping/admission and shutdown tests with shortened injected deadlines. Assert production's total is exactly 120 seconds, final-backup work stops before the reserved cancellation window, disabled tools remain paused, and no download/update occurs synchronously in an event hook.
- [ ] Run failing tests. disks_mounted marks availability and queues enabled initialized tools. stopping closes admission. stopping_svcs drains/cancels and attempts bounded final backups. Array restart preserves initialized RAM as authoritative. Don't touch share paths until the mount guard succeeds again.
- [ ] Test uninstall while a worker is running, final-backup failure, and leftover disabled legacy jobs. Preserve live installations, generations, settings, and legacy scripts. A final-backup failure is visible but never triggers destructive cleanup or legacy reactivation.
- [ ] Test deduplicated backup/update/migration notifications, notification delivery failure, and safe status output containing no fixture credentials/prompts. Keep transient notification state in RAM and successful-backup/update evidence off flash. Log notification delivery errors explicitly.
- [ ] Check captured stdout, stderr, notification bodies, and diagnostic logs for fixture secrets. TOML parser errors can include input snippets: preserve the cause internally but format only safe location/error context in diagnostics.
- [ ] Run `cargo test --locked --features test-support --test lifecycle` plus processes/migration tests. Confirm the fixture writer is gone before the shutdown test accepts completion, then commit.

Deliverable: Lifecycle integration that doesn't stall emhttp with normal long-running work or claim a cancelled writer has exited without checking.

### Task 8: Native settings page and request validation

**Files:** Create `unraid/CommandLineTools.page`, `unraid/include/request.php`, `unraid/include/actions.php`, `unraid/scripts/settings.js`, `tests/web_requests.rs`, and `tests/fixtures/unraid/request-harness.php`.

**Interfaces:** PHP `validate_action_request(array $request, string $expectedCsrfToken): array` returns only validated action/tool/settings/consent values. Production authentication is enforced through the supported Unraid request/session mechanism established in Task 1. The action handler invokes the fixed backend path with a `proc_open` argument array and bounded JSON input, never an interpolated shell string.

- [ ] Write Rust integration tests that invoke the PHP harness: unauthenticated session, GET mutation, invalid/missing CSRF, invalid tool/action/interval, oversized input, unexpected path/program fields, and shell syntax. Assert rejected requests start zero backend jobs.

  ```rust
  assert_eq!(missing_csrf_response_status, 403);
  assert!(recorded_backend_invocations.is_empty());
  ```

- [ ] Add accepted-request tests for settings, Backup Now, Initialize/Restore, and Update Now with automatic updates ON/OFF. Verify explicit fresh Codex consent reaches the backend as a boolean rather than an inferred default.
- [ ] Run failing tests using PHP CLI. Implement the request allowlist and fixed backend invocation. Do not weaken validation when Unraid includes/session state are absent. Test stubs are fixture-only and aren't packaged.
- [ ] Render per-tool enable/Automatic updates, backup interval, installed/cached/daemon versions, readiness, last-success times, errors, and restart requirements. Poll tracked status/jobs. Show credential-free terminal authentication instructions. Use native labels, keyboard-operable controls, escaped text, and an announced status area.
- [ ] Explain Disable's persistence consequence in the UI. Keep Initialize distinct from enabling an already initialized tool. Mark unknown/manual versions accurately and distinguish successful installation from failed persistence.
- [ ] Run `cargo test --locked --features test-support --test web_requests` and `php -l` on each PHP source/harness. Expect all pass. Actual Unraid login/CSRF enforcement and native page rendering remain server gates, then commit.

Deliverable: A settings page with tested request boundaries and no terminal/token collection.

### Task 9: Package, documentation, and draft release automation

**Files:** Create `packaging/ai-cli-manager.plg.in`, `packaging/build.sh`, `assets/ai-cli-manager.svg`, `tests/packaging.rs`, the two workflows, README/LICENSE/CHANGELOG, and migration/recovery/server-validation docs.

**Interfaces:** `packaging/build.sh VERSION OUTPUT_DIRECTORY` builds a package tree containing the binary, PHP page, event entry points, and filtered Plugin Manager post-hook. Produce `ai-cli-manager-VERSION-x86_64-1.txz`, `ai-cli-manager.plg`, and SHA256SUMS under OUTPUT_DIRECTORY. No installpkg/plugin invocation runs against the developer host.

- [ ] Write tests that inspect the package list, executable modes, fixed runtime paths, SHA256, lifecycle handlers, and absence of fixtures, test-root support, tokens, or vendor CLI packages. Inject a fake release executable/package tool where needed. Reject unsafe version/output paths before writing.

  ```rust
  assert!(!elf_has_interpreter);
  assert!(!release_accepts_fixture_root);
  assert!(archive_entries.iter().all(|path| !path.starts_with("tests/")));
  ```

- [ ] Run failing tests. Build the approved musl release with the matching musl C compiler for bundled SQLite. Check ELF architecture, interpreter, and shared-library requirements. Build a separate release without test-support and prove fixture overrides are rejected.
- [ ] Use plugin basename/name ai-cli-manager consistently so cron registration finds `/boot/config/plugins/ai-cli-manager/*.cron`. Include SHA256, changelog, min 7.3.4, max 7.99.99, and explicit runtime rejection of Unraid major 8. Test 7.3.3, 7.3.4, a later 7.x version, 8.0.0 prerelease, and 8.0.0. Only 7.3.4 is initially labelled server-tested.
- [ ] Keep package install free of synchronous CLI downloads, initialization, or takeover. Filter the post-hook by successful plugin registration. Removal calls the bounded backend uninstall path before removing manager components.
- [ ] Document settings defaults, manual authentication, approved target maintenance, unknown manual versions, online consistency limits, capacity overhead from full generations plus legacy backup, independent takeover, disable behavior, and bounded shutdown limitations. Document safe export of a committed generation to legacy layout and explicit operator reactivation of original jobs.
- [ ] Add the MIT license with the proposed attribution `Copyright (c) 2026 unraid-ai-cli-manager contributors`. Retain required dependency notices in release artifacts. Create an original SVG icon without vendor marks. Put project/support links in package/docs.
- [ ] CI runs fmt, clippy, Rust tests including PHP request tests, and package/ELF checks. Provision missing approved build/test tools only in its ephemeral runner. Pin external workflow actions by reviewed immutable revisions during implementation. The release workflow is workflow_dispatch only and creates drafts. It must not publish a public beta or submit to CA automatically.
- [ ] Run `cargo test --locked --features test-support --test packaging`, the full Rust suite, PHP syntax checks, and package build/inspection. Expect all pass, then commit.

Deliverable: Reviewable desktop package artifacts and draft automation, without installing them anywhere.

### Task 10: Integrated desktop acceptance and server handoff

**Files:** Create `tests/end_to_end.rs`. Complete `docs/server-validation.md` and README links to artifacts/release gates. Fix only defects found in the implemented scope.

**Interfaces:** Exercise the existing CLI/job/status/PHP/package boundaries. Don't introduce another orchestration interface for acceptance tests.

- [ ] Write an end-to-end fixture sequence: plugin install stays unmanaged; initialize Claude; preserve Codex legacy jobs; initialize Codex; make state changes; back up; simulate reboot RAM loss; restore offline; simulate stop/start with newer RAM state; validate RAM remains authoritative; uninstall while preserving state/settings.

  ```rust
  assert_eq!(restored_state, last_committed_state);
  assert_eq!(state_after_same_boot_restart, newer_ram_state);
  assert_eq!(settings_after_uninstall, settings_before_uninstall);
  ```

- [ ] Add an update failure sequence proving previous selection survives, a successful install followed by persistence failure is reported separately, and a newer unapproved manual version still persists without automatic downgrade. Verify last-success status stays tied to committed evidence.
- [ ] Run the new tests failing first where behavior is absent. Complete the smallest missing integration, then run fmt, clippy, all Rust tests, PHP syntax/request checks, and the release package build once. Re-run only relevant checks after any further fix.
- [ ] Write actual command results and tool versions into the desktop handoff. Don't claim real Unraid behavior from mocks. Resolve all desktop failures before pushing completed work. Preserve unrelated untracked `.serena` or local tool state rather than bundling it accidentally.
- [ ] Push the completed desktop repository and provide its URL for server review. Don't install/migrate production as part of that push. No public release, live schedule edit, or production-data operation is part of this desktop plan.
- [ ] Leave the actual server matrix explicitly pending: cold boot, hot install, offline restore, repeated stop/start, active-session migration, representative backup size/time, sandbox behavior and opt-in, updater OFF across startup/daemon paths, cron after registration, upgrade, uninstall/reinstall, rollback, and descendant cancellation.
- [ ] Public beta requires recorded server evidence, especially Codex automatic-update OFF for approved versions. Only then generate the root beta `.plg` URL, plugin listing XML and `ca_profile.xml`, run Community Applications Validate and Scan, and request manual review. Keep submission/publishing as separate authorized work.

Deliverable: Desktop acceptance evidence, a GitHub-ready repository, and an honest server-validation checklist.

## Self-review coverage

| Requirement | Owning tasks |
| --- | --- |
| D-01 independent migration / unrelated schedules | 6, 10 |
| D-02 explicit Initialize / inert package install | 5, 8, 9 |
| D-03 disabled tool behavior / no legacy reactivation | 5, 7, 8 |
| D-04 approved versions and shared manual/automatic backend | 5, 9 |
| D-05 external links, package links, runtime exclusions | 3, 4 |
| D-06 immutable snapshots, capacity, pruning, legacy retention | 3, 6 |
| D-07 notifications and secret-free diagnostics | 7, 8 |
| D-08 Rust/backend tests and PHP integration | 1–10 |
| D-09 exact dependency approval / reproducible lockfile | 1, 9 |
| D-10 MIT manager code / vendor separation | 9 |
| D-11 unapproved manual versions / no automatic downgrade | 5, 10 |
| SQLite consistency, retries, deletion, permissions | 4 |
| Worker concurrency, shutdown budget, detached descendants | 2, 7 |
| Installer integrity, rollback, retained active packages | 4, 5 |
| Native updater suppression, daemon restart policy, sandbox consent | 5, 8, 10 |
| Mount guards, cron registration, hot install, uninstall | 3, 7, 9 |
| Web request/session/CSRF validation | 1, 8, 10 |
| Packaging, draft CI/releases, docs, recovery, CA gates | 9, 10 |

All task interfaces use the shared types above. No task requires a generic storage abstraction, daemon, HTTP service, or new public CLI verb. The five review-focus cases each have a named owner and test assertions.

## Approval and known limits

The user approved preparing the shortlist and this plan. Implementation and dependency installation haven't started. Review the proposal's exact crate versions/features and its runtime-tool amendment before creating Cargo files. Also review the proposed fixture boundary, internal CLI actions, and worker supervisor mechanism.

The actual production scripts/schedules/package links, Unraid auth behavior, musl executable compatibility, and representative backup timings remain unverified. The source references on upstream master aren't proof of the installed 7.3.4 interfaces. If detached writer termination or the 120-second contract cannot be established on the supported host, keep that release gate open and report it plainly.

Choose native execution or explicitly delegated task/review execution after review. This plan itself doesn't dispatch agents or authorize production changes.
