# Unraid AI CLI Manager decisions

**Discussed:** 2026-11-11
**Status:** Interview decisions captured. Implementation hasn't started.
**Repository:** https://github.com/GoogilyBoogily/unraid-ai-cli-manager
**Decisions captured:** 11

This records the desktop handoff and the choices made during its follow-up interview. The original handoff still applies wherever this record doesn't explicitly amend it. No existing ADRs or implementation were present during inspection.

## Working boundaries

- Build and test on the desktop with injectable filesystem roots, fixtures, and mocked Unraid commands. Tests must guard against access to real production paths.
- Don't install anything on production, change its schedules, or migrate its data during desktop development.
- Don't copy credentials, histories, conversations, databases, or CLI home directories to the desktop or repository.
- Deliver source, tests, package automation, documentation, an original icon, and a draft release workflow. Push the completed desktop work for server-side review before installation and migration testing.
- Vendor CLIs come from official sources. Don't bundle them or imply vendor endorsement.

## User decisions

### D-01: Migrate each tool independently

**User Decision.** Each tool has its own migration validation, schedule takeover, and rollback.

- Successful Claude migration disables only its three legacy jobs. Codex migration handles its own three jobs separately.
- If Codex migration fails, its legacy jobs remain enabled and a successful Claude takeover stays in place.
- Preserve unrelated schedules in both schedule files and in generated cron.
- This amends the handoff's combined six-job takeover. All six jobs are disabled only after both tools have completed takeover.
- Rationale: Independent enablement shouldn't make one tool's failure block the other.
- Code context: Both supplied scripts have separate persistence roots, restore markers, and lock files. The six wrappers only invoke their respective restore/backup commands.

### D-02: Require explicit first-time initialization

**User Decision.** Installing the plugin leaves both tools unmanaged. Initialize/Adopt performs the selected tool's migration, restore, or fresh installation, then enables management after success.

- Plugin installation alone doesn't alter CLI update policy or legacy UserScripts schedules.
- Fresh Codex installations still require explicit consent before configuring unsandboxed host execution with on-request approvals.
- Rationale: Takeover should be deliberate and use the planned Initialize/Restore action.
- Affects: Installation defaults, settings actions, initialization jobs, and migration activation.

### D-03: Disabling a tool pauses all management

**User Decision.** Disable stops automatic restore, backups, and updates for that tool.

- Preserve its live CLI, saved generations, and settings.
- Preserve native unattended-updater suppression.
- Leave legacy jobs disabled. Don't silently hand control back to UserScripts.
- The separate Automatic updates toggle remains the control for pausing updates while retaining persistence.
- Rationale: The tool enable toggle controls management as a whole.
- Risk acknowledged in the interview: RAM changes made without further backups won't survive reboot.

### D-04: Automatic updates use approved version targets

**User Decision.** Each manager release identifies the Claude and Codex versions approved for automatic updates. New vendor versions wait for manager approval.

- Daily checks target approved versions rather than following every upstream release immediately.
- Manual vendor terminal updates remain available.
- Untested manually installed versions show Needs attention.
- Rationale: Package-layout and unattended-updater guarantees need a defined set of validated releases.
- Affects: Version metadata, desired-version resolution, compatibility status, release maintenance, and server validation.
- D-11 defines persistence and downgrade behavior for untested manually installed versions.

### D-05: Preserve external symlinks without copying their targets

**User Decision.** External targets remain separately managed. Preserve durable symlinks without traversing them during persistence.

- Exclude runtime socket links as required by the original handoff.
- Report missing external targets. A missing required target blocks readiness.
- Don't expand the manager's restore ownership into arbitrary external paths.
- Rationale: Keep backup and restore ownership bounded.
- Affects: Copy behavior, validation, readiness, and recovery instructions.

### D-06: Automatically prune superseded manager snapshots

**User Decision.** After publishing a validated generation, retain current and previous and automatically delete older manager-created generations and abandoned staging data.

- Preserve the legacy migration backup.
- Never discard the only good generation to make room for a new backup.
- Retain packages required by running processes under the handoff's package-retention rules.
- This explicitly authorizes the future plugin's limited automatic retention behavior. No production deletion occurred during the interview.
- Rationale: Bound routine snapshot storage without manual cleanup.
- Affects: Publication, retention, interrupted-job cleanup, and capacity checks.

### D-07: Use native Unraid failure notifications

**User Decision.** Notify on backup failures, update/persistence failures, and blocked migration.

- Suppress repeated identical alerts.
- Keep detailed status and logs available.
- Rationale: Failures must be visible when nobody is watching the settings page, especially when installation succeeds but persistence fails.
- Affects: Error reporting and notification state.

### D-08: Use a Rust backend and Rust tests

**User Decision.** Rust replaces the proposed Bash backend and Python tests.

- Rust owns persistence, migration, installation/update orchestration, locking, and worker lifecycle.
- Retain PHP as the thin Unraid settings-page integration.
- Run Rust unit and fixture-based integration tests with Cargo.
- Package a compiled executable in the Slackware archive. Don't require a Rust compiler on Unraid.
- Minimal event/package entry points may invoke that executable. Vendor installer scripts remain external programs.
- Rationale: The user requested a more strongly typed backend and Rust tests.
- Affects: Source layout, build/release automation, runtime compatibility checks, test fixtures, and UI/backend integration.
- Local observation: Desktop rustc and Cargo report version 1.96.0. No target binary was built.

### D-09: Allow focused Rust dependencies with prior review

**User Decision.** Focused crates for structured configuration and Unix operations are permitted in principle.

- Present the exact crate list and rationale before adding dependencies.
- No individual crate has been approved or added yet.
- Rationale: Use appropriate typed libraries without imposing a standard-library-only constraint.
- Affects: Cargo manifest, lockfile, native linking requirements, and build tooling.

### D-10: License manager code under MIT

**User Decision.** Use the MIT license for the manager code.

- Vendor CLI licensing remains separate.
- The exact copyright attribution still needs to be filled in when the license file is created.
- Rationale: The user selected the short permissive license option.
- Affects: Root LICENSE, package notices, and Community Applications submission metadata.

### D-11: Continue validated persistence for untested manual versions

**User Decision.** An untested version installed through a manual vendor command may still be backed up and restored when package and state validation succeeds.

- Show Needs attention for the unsupported version. That warning alone doesn't suspend persistence.
- Never automatically downgrade to an approved version.
- All other safety checks still apply, including storage availability, initialization, active-session conflicts, package links, and database validation.
- Successful persistence doesn't prove unattended-updater suppression for that version.
- Rationale: Preserve valid user state without making an unsupported compatibility claim or automatically reversing a manual installation.
- Affects: Version classification, backup/restore eligibility, update version selection, status, and regression tests.

## Existing production behavior supplied as reference

The user pasted both persistence scripts. These observations describe that supplied source, not a fresh inspection of the production host.

| Tool | Live state | Persistent root | Restore marker | Operation lock |
| --- | --- | --- | --- | --- |
| Claude | `/root/.claude`, `/root/.claude.json` | `/mnt/user/appdata/claude-code` | `/root/.claude-persist-restored` | `/var/run/claude-persist.lock` |
| Codex | `/root/.codex` | `/mnt/user/appdata/codex` | `/root/.codex-persist-restored` | `/var/run/codex-persist.lock` |

- Both expose `restore`, `backup`, and `status`. Both keep live state in RAM and guard share access with `mountpoint -q /mnt/user` plus an appdata-directory check.
- Both treat RAM state as authoritative after their boot-local marker exists. Repeated array stop/start must preserve this intended behavior.
- Claude selects a versioned executable under `/root/.local/share/claude/versions`, merges JSON with live values winning, and deduplicates history lines.
- Codex uses the full `packages/standalone` tree, including helpers, and SQLite `.backup` for active databases. Its history is merged on restore.
- The wrappers add no behavior beyond invoking script commands. Their schedule file contents and generated cron entries haven't been inspected.
- Legacy failure handling is insufficient: failed install/copy operations can still lead to a restore marker, and transfer/database failures can still lead to an overall success message.
- Codex excludes SQLite paths from rsync deletion and separately copies only existing live databases. Deleted live databases can therefore remain in the legacy cache. New generations must omit them.
- Codex restores its package before the broader state copy. The replacement must validate selectors and package links explicitly.

## Original handoff constraints that remain in force

- One combined plugin with independently enabled root host-native Claude and Codex installations. No Docker execution, embedded terminal, or multi-user support in v1.
- Automatic updates default ON per tool. Check daily at 04:00 server time. Update Now remains available with automatic updates ON or OFF.
- Backups default to every 15 minutes, with 5/15/30/60-minute choices.
- Preserve authentication and user configuration. Don't terminate active CLI work or automatically update/restart a running Codex daemon. Show daemon version and pending restart separately.
- Disable native unattended updates for managed installations. Claude uses `DISABLE_AUTOUPDATER=1`, preserving manual updates. Verify every supported Codex update path, including startup, in-app, installer metadata, and daemon behavior.
- Preserve persistence-root paths. Stage on the same filesystem, validate, publish immutable generations atomically, and restore only committed generations. Only successful publication updates last-successful-backup status.
- Missing live sources are errors. Check capacity before backup. Don't use shared inodes in ways that permit committed snapshots to change.
- Use bounded SQLite backup retries and `PRAGMA quick_check`. Each database is individually consistent. Online backups aren't a transaction across all application files.
- Enforce private directories and sensitive files with umask 077, directory mode 0700, and known sensitive-file mode 0600 while preserving executable permissions.
- Preserve tool-specific exclusions and required package helpers. Validate restored package links. Don't execute a cached package through links into a live installation.
- Reuse legacy locks during takeover. Locks serialize manager work, not application writes. Conflicting live state without valid initialization requires Needs attention rather than destructive restore or backup.
- Package a versioned Slackware `.txz` with a SHA256-verified `.plg`, changelog, support links, and compatibility bounds. Tested minimum is Unraid 7.3.4, with Unraid 8 excluded.
- Check required tools without replacing system libraries. Store manager settings under `/boot/config/plugins/ai-cli-manager`. Keep vendor packages, credentials, snapshots, and histories off flash.
- `disks_mounted` only marks availability and queues initialization. `stopping` rejects new jobs. `stopping_svcs` drains/cancels workers and attempts final backup within a 120-second total budget. Storage-writing descendants must terminate before the handler returns.
- Don't download or update CLIs inside synchronous event hooks. Reconcile cron after successful Plugin Manager registration through a filtered post-hook, and at array start and settings changes.
- Downloads use private temporary files, HTTP success checks, bounded timeouts, noninteractive installation, and vendor integrity verification. Resolve desired versions first, skip identical versions, retain previous package/selection, validate, roll selection back on failure, and persist successful installs immediately.
- Report installation success separately from persistence failure. Don't promise rollback of an already-running process or delete its packages.
- Preserve the backend interface `ai-cli-manager status|restore|backup|install|update --tool claude|codex|all`, with JSON status and tracked background jobs. Distinguish success, busy, storage waiting, restart required, and failure.
- Use Unraid authentication/CSRF checks, fixed action allowlists, safe argument passing, and secret-free diagnostics. Give terminal authentication instructions without collecting tokens.
- Migration verifies expected legacy scripts and markers, journals changes, acquires legacy locks, validates a generation, and updates only matching jobs in both `/boot/config/plugins/user.scripts/schedule.json` and `/tmp/user.scripts/schedule.json`, plus their generated cron entries.
- Preserve original scripts and document exporting a committed generation into legacy layout. Roll back schedule changes when that tool's takeover fails.
- Uninstall rejects new work, drains workers, attempts bounded final backup, and preserves live installations, persisted state, and settings. Don't silently reactivate UserScripts.

## Implementation research and server release gates

These are facts to establish, not user decisions to infer.

- Inspect actual production schedules, package links, permissions, and dependencies once read-only SSH works. The helper failed because its credential keyring couldn't unlock, both inside and outside the sandbox. Don't put credentials in chat or the repository.
- Determine the Rust target and native-linking requirements against actual Unraid. Verify how manager workers and all storage-writing descendants are cancelled within the shutdown budget.
- Research the exact Rust crate list, configuration-preserving edits, SQLite integration, and notification interface before proposing dependencies.
- Establish how legacy database copies are distinguished from stale deleted data during live-state adoption. Validate script fingerprints without requiring production data on the desktop.
- Measure capacity and backup duration against representative state. The handoff reports approximately 1.2 GB of live Codex state, which wasn't measured during this interview.
- Verify current Unraid hook/post-hook arguments, cron registration, web authentication, and CSRF conventions against supported Unraid.
- Prove Codex automatic-update OFF for every approved supported release and a running daemon. Unproven OFF behavior blocks public release.
- Test cold boot, hot installation, offline restore, repeated stop/start, migration with active sessions, sandbox consent/limitations, cron activation, upgrades, uninstall/reinstall, and rollback on actual Unraid. Desktop mocks don't establish these properties.
- Desktop tests must cover storage/dependency/source failures, interrupted copies/publication, SQLite busy/corruption, disk full, retention, links, exclusions, permissions, secret-free status, both schedule copies, unrelated jobs, update modes/rollback/persistence failures, shutdown, concurrency, uninstall, and web validation.
- After server validation, publish a beta raw `.plg` URL, add listing XML and root `ca_profile.xml`, provide support/project links, run Community Applications Validate and Scan, and submit for manual review.

## Planning follow-up

The user approved preparing a dependency shortlist and implementation plan. Both are now written for review:

- [Rust dependency shortlist](../rust-dependency-shortlist.md)
- [Desktop implementation plan](../../superpowers/plans/2026-11-11-ai-cli-manager.md)

Their exact dependency versions, runtime-tool amendment, and proposed implementation mechanisms remain proposals until reviewed. They don't add accepted decisions to D-01 through D-11 by themselves.

## References

- [Unraid event dispatcher](https://raw.githubusercontent.com/unraid/webgui/master/sbin/emhttp_event)
- [Unraid cron registration](https://raw.githubusercontent.com/unraid/webgui/master/sbin/update_cron)
- [Unraid Plugin Manager](https://raw.githubusercontent.com/unraid/webgui/master/emhttp/plugins/dynamix.plugin.manager/scripts/plugin)
- [Claude installation and update controls](https://code.claude.com/docs/en/setup)
- [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Community Applications submission requirements](https://ca.unraid.net/submit/help)
- [Community Applications repository XML](https://ca.unraid.net/submit/help/repository-xml)
- [Rust platform support](https://doc.rust-lang.org/rustc/platform-support.html)
- [Cargo tests](https://doc.rust-lang.org/cargo/guide/tests.html)
- [MIT license](https://opensource.org/license/mit)

Upstream pages were consulted during the interview. The repository XML page couldn't be fetched. Recheck interfaces during implementation rather than treating current upstream master as proof of Unraid 7.3.4 behavior.
