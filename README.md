# Unraid AI CLI Manager

One Unraid plugin for root's host-native Claude Code and Codex installations. The backend and its tests are Rust. PHP provides the native Settings page, and shell scripts only enter lifecycle hooks or build packages.

**Desktop preview. Server validation is pending.** This repository hasn't installed or migrated anything on production. The package targets x86_64 Unraid 7.3.4 through major version 7. Unraid 8 and prerelease OS versions are excluded. Compatibility bounds aren't a claim that every accepted version has been tested.

The approved vendor catalog is empty until real server testing passes. You can adopt an existing valid installation and exercise backup/restore during supervised validation. Fresh installs and manager updates fail with `unapproved_target` until a reviewed release includes a server-validated target. Codex updater OFF behavior blocks public release.

## Behavior

- Install the plugin, then explicitly Initialize/Adopt each tool. Installation alone enables nothing.
- Each tool has independent Enabled and Automatic updates settings. Automatic updates defaults ON, with checks at 04:00 server time. Update Now uses the same backend and works with automatic updates OFF.
- Backups run every 15 minutes by default. Choose 5, 15, 30, or 60 minutes.
- Live state stays in RAM. Backups use the existing appdata roots, with `current` selecting an immutable completed generation. Keep current and previous generations, plus the retained legacy migration backup.
- Disabling a tool pauses scheduled work. It preserves installed tools, saved state, settings, and native updater suppression. It doesn't reactivate UserScripts.
- Manual vendor update commands remain available. Unknown versions need attention but may continue validated persistence. The manager never automatically downgrades them.
- Running user sessions aren't killed. A running Codex daemon isn't updated or restarted. Status reports running daemon versions and pending restarts separately.

| Tool | Live state | Persistent root | Launcher |
| --- | --- | --- | --- |
| Claude Code | `/root/.claude`, `/root/.claude.json` | `/mnt/user/appdata/claude-code` | `/root/.local/bin/claude` |
| Codex | `/root/.codex` | `/mnt/user/appdata/codex` | `/root/.local/bin/codex` |

Both tools also get `/usr/local/bin` convenience links. No credentials, histories, databases, snapshots, or vendor packages belong on the flash drive. Only manager settings, migration metadata, and the manager's own package live there.

## Desktop development

Use Rust 1.96.0 and an x86_64 Linux desktop with rsync, PHP CLI, Node.js, Python 3, GNU tar, xz, binutils, and musl-tools. The pinned toolchain supplies rustfmt and Clippy. No production SSH connection is required.

```bash
rustup target add x86_64-unknown-linux-musl
cargo fmt --all --check
cargo clippy --locked --all-targets --features test-support -- -D warnings
cargo clippy --locked --all-targets --no-default-features -- -D warnings
cargo test --locked --features test-support
cargo test --locked --no-default-features
php -n -l unraid/include/actions.php
node --check unraid/scripts/settings.js
packaging/build.sh
cargo test --locked --features test-support --test packaging -- --include-ignored
```

If PHP isn't on PATH, set `AI_CLI_MANAGER_PHP` to its CLI executable. For a relocated musl compiler, set `CC_x86_64_unknown_linux_musl` and `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER` to its wrapper before building.

Fixtures use private temporary roots beneath `/tmp` and synthetic tool packages. `test-support` is required for fixture-root/program overrides. The production build rejects `AI_CLI_MANAGER_TEST_ROOT`, and the package builder checks that guard. Tests don't read or copy production CLI homes. Never package a test-support executable.

`packaging/build.sh` produces `dist/ai-cli-manager-0.1.0-x86_64-1.txz`, `dist/ai-cli-manager.plg`, and `dist/SHA256SUMS`. It checks static ELF linkage, normalizes archive metadata, and embeds the package SHA256 in the manifest. It doesn't bundle vendor CLIs or replace system libraries. Runtime requirements are rsync, curl, mountpoint, bash/sh, logger, and Unraid's notify/update_cron helpers. JSON/TOML parsing, SQLite, locks, and time limits are handled by Rust and privately linked SQLite.

Actual desktop results and review corrections are recorded in [desktop validation](docs/desktop-validation.md). Release packages retain dependency license/notice texts under `/usr/share/doc/ai-cli-manager/licenses`.

## Server review and installation

First review the pushed source and [server validation gates](docs/server-validation.md) on the server. Don't install a desktop artifact as if it had passed those gates.

After supervised validation and a reviewed beta release, install the published raw `ai-cli-manager.plg` URL through Unraid Plugins > Install Plugin. The release's root manifest must reference its immutable release package and checksum. Open Settings > AI CLI Manager, then follow [migration](docs/migration.md) for an existing UserScripts setup. The Plugin Manager post-hook reconciles cron after registration, and array start/settings changes reconcile it again.

Authenticate in a root terminal using `claude` or `codex login`. The settings page never asks for tokens. Existing credentials and user configuration are retained. Fresh Codex installs leave execution policy alone unless you explicitly select unsandboxed host execution with on-request approvals. The documented initramfs sandbox limitation still needs testing on Unraid.

No Docker execution, embedded terminal, or multi-user support is included.

## CLI and status

```bash
ai-cli-manager status --tool all --json
ai-cli-manager backup --tool claude --json
ai-cli-manager restore --tool codex --json
ai-cli-manager update --tool all --json
```

The supported verbs are `status`, `restore`, `backup`, `install`, and `update`. `install` uses the managed update path after initialization. First-time setup is explicit in Settings. Responses are JSON, including when `--json` is omitted. Long operations return job identifiers, and status exposes each tool's last job. Private `__internal` commands serve hooks and the page and aren't a supported automation interface.

Status separates disabled, waiting for storage, busy, ready, restart required, and needs attention. A successful installation with failed persistence is a failed job with `installation: succeeded` and `persistence: failed`. It doesn't claim the new version survived reboot. Error messages omit command output, credentials, and user file contents.

## Persistence limits and recovery

Never create anything beneath `/mnt/user` before the share mount succeeds. Storage jobs pin checked directory handles so a mount-path race can't redirect backup writes onto rootfs. A missing live source or initialization marker blocks backup.

Each SQLite database uses the backup API with bounded busy retries and `quick_check`. Each database is consistent individually. An online backup isn't a transaction across every application file. Regular files can change while the application runs, and locks serialize manager jobs rather than application writes. External state links remain links and their targets aren't included. Package links must resolve inside their package tree. Runtime links into `/tmp` and `/run`, sockets, caches, PID files, and runtime locks are excluded.

Full copies need room for a new generation while retaining the previous two. The capacity preflight includes a 64 MiB reserve and may conservatively reject a backup. It doesn't reserve space against other writers. Copy or validation failure preserves the committed selector. No shared writable inodes are used between generations.

Shutdown rejects new work, cancels workers, and attempts final backups inside a 120-second total budget. Worker supervisors retain locks while descendants remain. A kernel-blocked process may outlive that budget, which produces an explicit drain failure rather than a success claim. Validate actual Unraid shutdown behavior before release. RAM installations remain after array stop, but array-backed workspaces may be unavailable.

Uninstall drains work, attempts the bounded final backup, clears only the manager schedule, and removes manager package files. It preserves installations, appdata, settings, original scripts, and legacy schedules in their disabled state. See [recovery and legacy export](docs/recovery.md).

## Releases and Community Applications

CI builds reviewable desktop artifacts. The manual release workflow creates a draft prerelease, never an automatic public release. After the [server gates](docs/server-validation.md) pass, add verified vendor targets, build a beta, and publish its manifest at the repository root. Then add listing XML and `ca_profile.xml`, run Community Applications Validate and Scan, and request manual review. These listing files are deliberately deferred until server validation.

[Project](https://github.com/GoogilyBoogily/unraid-ai-cli-manager) · [Support/issues](https://github.com/GoogilyBoogily/unraid-ai-cli-manager/issues) · [Upstream contracts](docs/upstream-contracts.md)

Manager code and original icon are MIT licensed. Vendor CLIs download from official sources and retain their own licenses. This project isn't endorsed by Anthropic, OpenAI, or Lime Technology.
