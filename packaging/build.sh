#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\([0-9][0-9.]*\)"$/\1/p' Cargo.toml | head -n1)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Invalid package version' >&2; exit 1; }
for program in cargo install tar xz sha256sum readelf python3; do command -v "$program" >/dev/null || { echo "Missing build tool: $program" >&2; exit 1; }; done
cargo build --locked --release --no-default-features --target x86_64-unknown-linux-musl --bin ai-cli-manager
binary=target/x86_64-unknown-linux-musl/release/ai-cli-manager
if readelf -l "$binary" | grep -q 'INTERP' || readelf -d "$binary" | grep -q 'NEEDED'; then echo 'Release binary must be fully static' >&2; exit 1; fi
mkdir -p dist
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
printf 'ai-cli-manager-fixture-v1\n' > "$stage/.ai-cli-manager-fixture"
if AI_CLI_MANAGER_TEST_ROOT="$stage" "$binary" status --json > /dev/null 2> "$stage/fixture-error"; then echo 'Release binary accepted a fixture override' >&2; exit 1; fi
grep -q 'fixture_override_forbidden' "$stage/fixture-error" || { echo 'Release fixture guard did not reject the override' >&2; exit 1; }
rm -- "$stage/.ai-cli-manager-fixture" "$stage/fixture-error"
install -Dm755 "$binary" "$stage/usr/local/sbin/ai-cli-manager"
plugin="$stage/usr/local/emhttp/plugins/ai-cli-manager"
install -Dm644 unraid/CommandLineTools.page "$plugin/CommandLineTools.page"
for name in actions.php request.php; do install -Dm644 "unraid/include/$name" "$plugin/include/$name"; done
install -Dm644 unraid/scripts/settings.js "$plugin/scripts/settings.js"
install -Dm644 assets/ai-cli-manager.svg "$plugin/images/ai-cli-manager.svg"
for event in disks_mounted stopping stopping_svcs; do install -Dm755 "unraid/event/$event" "$plugin/event/$event"; done
install -Dm755 unraid/plugin-post-hook "$stage/usr/local/emhttp/plugins/dynamix.plugin.manager/post-hooks/ai-cli-manager"
install -Dm644 LICENSE "$stage/usr/share/doc/ai-cli-manager/LICENSE"
install -Dm644 packaging/approved-versions.json "$stage/usr/share/doc/ai-cli-manager/approved-versions.json"
python3 packaging/collect-licenses.py "$stage/usr/share/doc/ai-cli-manager/licenses"
chmod 755 "$stage"
mkdir -p "$stage/install"
cat > "$stage/install/slack-desc" <<'DESCRIPTION'
ai-cli-manager: AI CLI Manager
ai-cli-manager:
ai-cli-manager: Root-native Claude Code and Codex persistence for Unraid.
ai-cli-manager: Explicit initialization, immutable generations, and tracked jobs.
ai-cli-manager: No vendor CLIs are bundled.
DESCRIPTION
package="ai-cli-manager-$version-x86_64-1.txz"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
export SOURCE_DATE_EPOCH
# Normalize metadata without changing the private state permissions enforced at runtime.
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner -C "$stage" -cJf "dist/$package" .
python3 - "$version" "$package" <<'PY'
import hashlib
import pathlib
import sys
version, name = sys.argv[1:]
archive = pathlib.Path('dist') / name
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
template = pathlib.Path('packaging/ai-cli-manager.plg.in').read_text()
pathlib.Path('dist/ai-cli-manager.plg').write_text(template.replace('@VERSION@', version).replace('@SHA256@', checksum))
pathlib.Path('dist/SHA256SUMS').write_text(f'{checksum}  {name}\n')
PY
printf 'Built dist/%s and dist/ai-cli-manager.plg\n' "$package"
