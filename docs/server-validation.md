# Server validation and release gates

Status: **NOT RUN**. Desktop fixtures don't establish any gate below. No production installation, schedule change, migration, or private state transfer was performed during desktop development.

Record commit, package SHA256, Unraid version, tool versions, date, operator, result, and secret-free evidence for every gate. The intended first host is Unraid 7.3.4 x86_64. Don't infer support from a desktop Linux pass.

| Gate | Required evidence | Result |
| --- | --- | --- |
| Cold boot | Mount guard never creates `/mnt/user` on rootfs, committed state and helpers restore offline, boot markers appear only after success | Pending |
| Hot installation | New installation stays inert, existing settings survive reinstall, cron activates after Plugin Manager registration | Pending |
| Independent migration | Claude then Codex adoption with live sessions, both schedule copies and generated cron, unrelated jobs preserved | Pending |
| Migration rollback | Interrupted phases and changed owned/unrelated schedule entries, recoverable journal and before-images | Pending |
| Stop/start | Repeated array stop/start, admission closes, workers and descendants exit, final backup stays within 120 seconds | Pending |
| Capacity and timing | Representative 1.2 GB+ Codex state, elapsed backup time, free space, peak RAM, full-copy reserve and disk-full failure | Pending |
| Codex execution | Reproduce initramfs `pivot_root: Invalid argument`, verify explicit fresh-install opt-in, preserve on-request approvals | Pending |
| Claude update policy | DISABLE_AUTOUPDATER suppresses unattended paths, manual `claude update` remains usable, active process restart status | Pending |
| Codex update policy — release blocker | Automatic updates OFF across CLI startup, in-app update paths, installer selection metadata, and an already-running daemon. Test daemon separately and after restart. Observe network/update attempts and package selections | Pending |
| Managed updates | Approved pinned target, identical version skip, unknown/manual newer version retained, installer failure rollback, install success with persistence failure | Pending |
| Running daemon | Report its actual version separately, never automatically replace/restart it, accurate pending restart state | Pending |
| Web security | Native authentication and CSRF with real Unraid PHP prepend/nginx, malformed requests rejected before backend actions | Pending |
| Upgrade/uninstall | Upgrade drains work, uninstall preserves settings/state/installations, reinstall and explicit legacy export rollback work | Pending |
| Notifications | One native warning for repeated failure, success resets deduplication, no secrets in logs/status | Pending |

## Approve a vendor target

Only after the matching tests pass, put the exact stable numeric version and `server_verified: true` into `packaging/approved-versions.json` for that tool. Recheck official installer/configuration interfaces and retain vendor integrity checks. Run all desktop checks again and review the catalog/evidence change. A version number alone isn't update-policy evidence.

The current empty catalog intentionally blocks fresh installs and managed updates. Existing supported package layouts can still be adopted and persisted for supervised tests. Don't flip the verification flag to make the UI green.

## Publish a beta

1. Complete the required gates and independent code review. Make server evidence reviewable without uploading private CLI state.
2. Run the draft release workflow on the reviewed commit. Inspect the static `.txz`, package SHA256, manifest compatibility bounds, changelog, and URLs.
3. Publish a beta only after review. Copy its generated manifest to root `ai-cli-manager.plg` on the release branch so the raw install/update URL exists and references the immutable release asset.
4. Add plugin listing XML and root `ca_profile.xml` with real project/support links. Follow the current [Community Applications submission requirements](https://ca.unraid.net/submit/help) and [repository XML contract](https://ca.unraid.net/submit/help/repository-xml).
5. Run Community Applications Validate, then Scan, and submit for manual review. Record the results. Don't advertise support beyond versions actually tested.
