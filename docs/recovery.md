# Recovery and rollback

Start with `ai-cli-manager status --tool all --json`. Keep diagnostics secret-free. Don't attach CLI homes, auth files, histories, databases, raw installer output, or configuration before-images to public issues.

## Backup or update failed

A missing mount, marker, source, helper, capacity reserve, invalid link, corrupt database, busy SQLite database, failed transfer, or failed validation prevents a successful backup. The prior committed generation remains selected unless publication itself completed. Only a successful publication updates `last-success.json`.

An installation can succeed while its immediate backup fails. Status then reports installation success separately from persistence failure. Repair storage/capacity and use Backup Now before rebooting. The manager retains previous live packages and restores the old launcher selection when candidate installation or validation fails. It can't roll back a process that's already running.

Unknown manually installed versions don't trigger an automatic downgrade. If their package and state validate, backup/restore can continue. Add a target to a manager release only after the actual server gates pass.

## Restore conflict

Restore reads only a completed generation. Without a valid boot marker, active processes or unexplained live state require attention. Stop the conflicting session yourself after saving its work, then inspect and preserve the live files. Don't delete RAM state or fabricate a restore marker to bypass the check.

A restore interrupted after its journal was written can retry using the same committed generation. Claude JSON merges with live values winning, and histories deduplicate exact lines. Codex databases restore from validated snapshots. External state targets must be restored separately by their owner.

If current is damaged, preserve it and the previous generation before making changes. With manager scheduling disabled and all manager jobs drained, an operator can atomically select the validated predecessor recorded by the current manifest. Validate that generation's manifest, package links, required helpers, and database `quick_check` first. This is a supervised recovery step, not an automatic UI action.

## Export a committed generation to the legacy layout

Don't overwrite the current persistence root. Choose a new private recovery directory on mounted storage and copy from a selected committed generation. These commands only export into a new destination. Substitute a verified generation directory for `GENERATION` and an unused protected path for `EXPORT`.

```bash
umask 077
mountpoint -q /mnt/user || exit 1
GENERATION=/mnt/user/appdata/claude-code/generations/VERIFIED-GENERATION-ID
EXPORT=/mnt/user/appdata/claude-code-legacy-export-UNIQUE
[ ! -e "$EXPORT" ] || exit 1
mkdir -m 700 "$EXPORT"
rsync -a -- "$GENERATION/root/.claude/" "$EXPORT/.claude/"
if [ -e "$GENERATION/root/.claude.json" ]; then cp -p -- "$GENERATION/root/.claude.json" "$EXPORT/.claude.json"; fi
mkdir -m 700 "$EXPORT/bin"
```

For Claude, read the version and relative executable from that generation's manifest. Verify the executable resolves inside the generation, then copy that executable into `$EXPORT/bin/claude` and write its version into `$EXPORT/bin/.version`. Don't execute it through a link into the RAM installation. Preserve executable permissions.

For Codex, use a separate empty export directory and copy `GENERATION/root/.codex/` to `EXPORT/.codex/` with `rsync -a`. The complete `packages/standalone` release tree and relative current selector must be present. Don't export only the `codex` executable. Run SQLite `quick_check` against each exported database and verify package helpers before considering the export usable.

Keep the original legacy script references with the export. The retained `legacy-migration-backup` is an additional before-image, not a replacement for exporting the newest committed generation.

After verifying the export, explicitly choose how to replace the legacy persistence root, preserve the manager generations, and re-enable each legacy schedule. Those steps can overwrite data and aren't automated. Don't run legacy backup jobs against the manager's generations layout. Never enable both schedulers for the same tool.

## Uninstall and reinstall

Uninstall stops admissions, drains workers, attempts final backups within 120 seconds, and clears the manager schedule before removing manager package files. It preserves RAM installations, appdata generations, legacy backup, settings, and original scripts. It doesn't silently restart legacy jobs. A drain or final-backup failure aborts removal and needs inspection.

Reinstalling retains settings. Existing initialized/enabled tools resume through normal array-start/hot-install handling. Tools never initialized remain inert. The Plugin Manager post-hook restores cron after successful registration.

A kernel-blocked storage writer may exceed the shutdown budget. The hook reports failure and its supervisor retains the legacy lock. Investigate storage on the actual host before allowing an unmount. Desktop process tests can't establish kernel/FUSE shutdown behavior.
