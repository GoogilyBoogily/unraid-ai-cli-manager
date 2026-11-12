# Migration from UserScripts

Desktop development doesn't run this migration on production. Use this procedure only during the supervised server phase, after reviewing the pushed commit and package.

## Before takeover

1. Confirm `/mnt/user` is mounted and appdata has room for an additional full copy plus the 64 MiB reserve. Codex state was approximately 1.2 GB on the reference host. Measure the actual installation.
2. Keep the original persistence scripts and the six wrapper directories. Compare their bytes with the reviewed files under `tests/fixtures/legacy`. A mismatch stops adoption rather than guessing which script is safe.
3. Let the legacy restore complete. Its boot marker must exist. Keep active CLI sessions running for the migration test, with work you can safely inspect afterward.
4. Save independent copies of both UserScripts schedule files and `customSchedule.cron`. Keep these on protected storage, not in a public issue or repository.

## Adopt one tool

Open Settings > AI CLI Manager and choose Initialize/Adopt for Claude Code or Codex. Adoption is independent per tool. The other tool's jobs remain unchanged until you adopt it separately.

The manager waits for its legacy lock, verifies reviewed scripts/wrappers and the restore marker, and refuses queued legacy processes. It journals owned schedule entries on flash and stores configuration before-images in protected appdata. It adopts already-restored RAM state and doesn't restore over active sessions.

It publishes a validated first generation before disabling exactly that tool's three entries in both schedule copies. It removes only those wrappers' generated cron lines, enables manager scheduling, and refreshes cron. Successful adoption retains the old layout under `legacy-migration-backup/`. Original scripts remain in place.

If takeover fails, rollback restores the owned schedule fields and policy/settings before-images. It preserves unrelated schedule edits. A concurrent edit to an owned field blocks rollback and reports `rollback_failed` rather than overwriting it. Review the specific conflict before retrying.

## Interrupted takeover

The journal is `/boot/config/plugins/ai-cli-manager/migration-TOOL.json`. Its phases are Prepared, Disabled, Committed, and RolledBack. It contains manager settings and owned schedule metadata, not CLI credentials. Configuration before-images live in the tool's `legacy-migration-backup/policy-*.json`.

After an interrupted migration, the next Initialize/Adopt attempt first rolls back a noncommitted journal and reports Needs attention. Review schedules and status, then retry explicitly. A committed journal prevents repeated takeover. Don't edit journal paths or phase fields to bypass validation.

If the previous job supervisor was lost, status refuses new work for that tool. On the server, inspect its recorded PID/start time and descendants before clearing any stale runtime state. Don't clear an active job selector merely to remove a Busy indicator. A normal reboot resets RAM job records and boot markers, while the migration journal remains available for recovery.

## Verify takeover

- Both copies of `schedule.json` show the selected tool's restore/backup/backup-on-stop jobs as disabled. Unrelated jobs are byte-for-byte equivalent in meaning.
- `customSchedule.cron` contains no generated line for those three wrappers. The manager cron appears after plugin registration.
- Backup Now succeeds, updates last successful backup, and retains only current/previous manager generations.
- Existing authenticated sessions still work. Root state and known sensitive files have private permissions.
- Stop/start and cold-boot tests restore from committed state without network access. Follow the complete server gate checklist before calling migration validated.

Disabling or uninstalling the manager doesn't reactivate the old jobs. Returning to UserScripts is an explicit recovery operation described in [recovery.md](recovery.md).
