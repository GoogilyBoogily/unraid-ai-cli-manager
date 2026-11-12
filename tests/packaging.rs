use std::{path::Path, process::Command};
#[test]
fn plugin_template_and_icon_are_valid_xml_with_bounded_hooks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let result = Command::new("python3")
        .args([
            "-c",
            r#"
import pathlib,xml.etree.ElementTree as E
root=E.parse('packaging/ai-cli-manager.plg.in').getroot()
assert root.tag=='PLUGIN' and root.attrib['name']=='ai-cli-manager'
files=root.findall('FILE')
package=next(f for f in files if f.find('SHA256') is not None)
assert package.attrib['Min']=='7.3.4' and package.attrib['Max'].startswith('7.')
assert package.findtext('SHA256')=='@SHA256@'
assert 'releases/download/v@VERSION@' in package.findtext('URL')
assert E.parse('assets/ai-cli-manager.svg').getroot().tag.endswith('svg')
for name in ['disks_mounted','stopping','stopping_svcs']:
    text=pathlib.Path('unraid/event',name).read_text()
    assert 'exec /usr/local/sbin/ai-cli-manager __internal' in text
    assert 'curl' not in text
"#,
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
#[ignore = "requires packaging/build.sh to produce static release artifacts"]
fn archive_has_only_manager_files_and_checksum_matches_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let result=Command::new("python3").args(["-c",r#"
import hashlib,json,pathlib,subprocess,tarfile,tempfile,os,xml.etree.ElementTree as E
root=E.parse('dist/ai-cli-manager.plg').getroot()
entry=next(f for f in root.findall('FILE') if f.find('SHA256') is not None)
name=pathlib.Path(entry.attrib['Name']).name
archive=pathlib.Path('dist',name)
assert hashlib.sha256(archive.read_bytes()).hexdigest()==entry.findtext('SHA256')
with tarfile.open(archive) as package:
    members=package.getmembers()
    assert all(m.uid==0 and m.gid==0 and not m.issym() and not m.islnk() for m in members)
    paths={m.name.removeprefix('./') for m in members}
    for required in ['usr/local/sbin/ai-cli-manager','usr/local/emhttp/plugins/ai-cli-manager/include/actions.php','usr/local/emhttp/plugins/dynamix.plugin.manager/post-hooks/ai-cli-manager','install/slack-desc']:
        assert required in paths, required
    assert not any(p.startswith(('root/','mnt/','boot/')) for p in paths)
    assert package.getmember('.').mode == 0o755
    assert 'usr/share/doc/ai-cli-manager/licenses/THIRD-PARTY-NOTICES.txt' in paths
    assert any('SQLite-public-domain.txt' in p for p in paths)
    binary=package.extractfile('./usr/local/sbin/ai-cli-manager').read()
with tempfile.TemporaryDirectory(prefix='ai-cli-package-',dir='/tmp') as directory:
    path=pathlib.Path(directory,'manager');path.write_bytes(binary);path.chmod(0o700)
    assert 'INTERP' not in subprocess.check_output(['readelf','-l',path],text=True)
    assert 'NEEDED' not in subprocess.check_output(['readelf','-d',path],text=True)
    pathlib.Path(directory,'.ai-cli-manager-fixture').write_text('ai-cli-manager-fixture-v1\n')
    result=subprocess.run([path,'status','--json'],env={**os.environ,'AI_CLI_MANAGER_TEST_ROOT':directory},capture_output=True,text=True)
    assert result.returncode!=0 and json.loads(result.stderr)['code']=='fixture_override_forbidden'
"#]).current_dir(root).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
