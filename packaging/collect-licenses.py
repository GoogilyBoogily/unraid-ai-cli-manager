#!/usr/bin/env python3
"""Retain license and notice texts from the exact Cargo-resolved source packages."""
import json
import pathlib
import shutil
import subprocess
import sys

output = pathlib.Path(sys.argv[1])
output.mkdir(parents=True, exist_ok=True)
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-unknown-linux-musl']))
nodes = {node['id']: node for node in metadata['resolve']['nodes']}
reachable = {metadata['resolve']['root']}
pending = list(reachable)
while pending:
    for dependency in nodes[pending.pop()]['dependencies']:
        if dependency not in reachable:
            reachable.add(dependency)
            pending.append(dependency)
index = ['Third-party components from Cargo.lock. Each component retains its upstream license.\n']
for package in sorted(metadata['packages'], key=lambda package: (package['name'], package['version'])):
    if package['source'] is None or package['id'] not in reachable:
        continue
    source = pathlib.Path(package['manifest_path']).parent
    texts = {path for path in source.rglob('*') if path.is_file() and path.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'COPYRIGHT', 'UNLICENSE'))}
    if package['license_file']:
        texts.add(source / package['license_file'])
    if not texts:
        raise SystemExit(f"Missing license texts for {package['name']} {package['version']}")
    destination = output / f"{package['name']}-{package['version']}"
    for text in sorted(texts):
        target = destination / text.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(text, target)
    if package['name'] == 'libsqlite3-sys':
        amalgamation = (source / 'sqlite3/sqlite3.c').read_text()
        position = amalgamation.index('blessing')
        start = amalgamation.rfind('/*', 0, position)
        end = amalgamation.index('*/', position) + 2
        (destination / 'SQLite-public-domain.txt').write_text(amalgamation[start:end] + '\n')
    index.append(f"{package['name']} {package['version']}: {package['license'] or 'see license file'}\n")
(output / 'THIRD-PARTY-NOTICES.txt').write_text(''.join(index))
