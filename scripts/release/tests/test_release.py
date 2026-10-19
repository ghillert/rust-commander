#!/usr/bin/env python3
"""Offline release rehearsal: versioning, branch/tag guards and resumable uploads."""
import base64
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
import os
import platform
import shutil
import subprocess
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release

MANIFEST = '[package]\nname = "rust-commander"\nversion = "0.7.0"\n\n[dependencies]\nfoo = "1"\n'
LOCK = 'version = 4\n\n[[package]]\nname = "foo"\nversion = "1.2.3"\nsource = "registry+example"\nchecksum = "abc"\n\n[[package]]\nname = "rust-commander"\nversion = "0.7.0"\ndependencies = ["foo"]\n'


class FakeGitHub:
    repo = "ghillert/rust-commander"
    prefix = "repos/ghillert/rust-commander/"

    def __init__(self):
        self.files = {"source": {"Cargo.toml": MANIFEST, "Cargo.lock": LOCK}}
        self.branch = None
        self.tag = None
        self.commit = None
        self.release = None
        self.assets = []

    def file(self, name, ref):
        return self.files[ref][name]

    def get(self, path, missing=False):
        if path.startswith("releases/tags/"):
            return self.release if self.release and not self.release.get("draft") else None
        if path.startswith("releases?"):
            return [dict(self.release, tag_name="v0.8.0")] if self.release else []
        if path.startswith("git/ref/heads/"):
            return {"object": {"sha": self.branch}} if self.branch else None
        if path.startswith("git/ref/tags/"):
            return {"object": {"sha": self.tag, "type": "commit"}} if self.tag else None
        if path.startswith("commits/"):
            return self.commit
        if "/assets?" in path:
            return copy.deepcopy(self.assets)
        raise AssertionError(path)

    def post(self, path, data):
        if data["ref"].startswith("refs/tags/"):
            self.tag = data["sha"]
            return {"object": {"sha": self.tag}}
        self.branch = data["sha"]
        return {"object": {"sha": self.branch}}

    def request(self, method, path, data=None):
        if method == "DELETE":
            self.assets = [a for a in self.assets if not path.endswith('/'+str(a['id']))]
            return
        self.assert_mutation(data)
        body = data["variables"]["input"]
        self.files["release"] = {f["path"]: base64.b64decode(f["contents"]).decode() for f in body["fileChanges"]["additions"]}
        self.branch = "release"
        self.commit = {"parents": [{"sha": "source"}], "files": [{"filename": n} for n in self.files["release"]],
                       "commit": {"message": body["message"]["headline"]+'\n\n'+body["message"]["body"],
                                  "verification": {"verified": True}}}
        return {"data": {"createCommitOnBranch": {"commit": {"oid": "release"}}}}

    def assert_mutation(self, data):
        assert data["variables"]["input"]["expectedHeadOid"] == "source"


class ReleaseTests(unittest.TestCase):
    @unittest.skipUnless(platform.system() == 'Linux', 'Debian package tools require Linux')
    def test_debian_prerelease_normalization_preserves_payload_and_upgrade_order(self):
        import package
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            (source / 'DEBIAN').mkdir(parents=True)
            (source / 'usr/bin').mkdir(parents=True)
            payload = source / 'usr/bin/rust-commander'
            payload.write_text('#!/bin/sh\necho fixture\n')
            payload.chmod(0o755)
            for version in ('0.8.0-rc.1', '0.8.0'):
                (source / 'DEBIAN/control').write_text(
                    f'Package: rust-commander\nVersion: {version}\nArchitecture: amd64\n'
                    'Maintainer: Test <test@example.test>\nDescription: Disposable release fixture\n')
                archive = root / 'fixture.deb'
                subprocess.run(['dpkg-deb', '--build', '--root-owner-group', str(source), str(archive)],
                               check=True, capture_output=True)
                package.normalize_deb(archive, version)
                actual = subprocess.check_output(['dpkg-deb', '-f', str(archive), 'Version'], text=True).strip()
                self.assertEqual(actual, package.debian_version(version))
            extracted = root / 'extracted'
            subprocess.run(['dpkg-deb', '-x', str(archive), str(extracted)], check=True)
            restored = extracted / 'usr/bin/rust-commander'
            self.assertEqual(restored.read_bytes(), payload.read_bytes())
            self.assertTrue(os.access(restored, os.X_OK))
            subprocess.run(['dpkg', '--compare-versions', package.debian_version('0.8.0-rc.1'), 'lt', '0.8.0'], check=True)

    def test_offline_publication_rehearsal_resumes_after_interrupted_upload(self):
        api=FakeGitHub(); release.prepare(api,'0.8.0','source','123')
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); directory=root/'dist/release';directory.mkdir(parents=True)
            shutil.copy2(release.ROOT/'jreleaser.json',root/'jreleaser.json')
            for target in release.TARGETS:
                assets={}
                for name in release.asset_names('0.8.0',target):
                    (directory/name).write_bytes(b'package fixture')
                    assets[name]={'sha256':release.digest(directory/name),'size':15}
                (directory/f'{target}.json').write_text(json.dumps({'version':'0.8.0','commit':'release','target':target,'assets':assets}))
            interrupted=False;uploaded=[]
            def command(args,**kwargs):
                nonlocal interrupted
                if args[0]=='git':
                    return subprocess.CompletedProcess(args,0)
                config=json.loads(Path(args[args.index('-c')+1]).read_text())
                paths=[Path(f['path']) for f in config['files']['artifacts']]
                if args[1]=='checksum':
                    checksum=root/'out/jreleaser/checksums/checksums_sha256.txt'
                    checksum.parent.mkdir(parents=True,exist_ok=True)
                    checksum.write_text(''.join(f'{release.digest(p)}  {p.name}\n' for p in reversed(paths)))
                elif args[1]=='release':
                    if api.release is None:
                        api.release={'id':1,'draft':True,'prerelease':False,'html_url':'https://example.test/draft','body':'original'}
                    for path in paths:
                        self.assertNotIn(path.name,uploaded)
                        uploaded.append(path.name)
                        api.assets.append({'id':len(uploaded),'name':path.name,'size':path.stat().st_size,'state':'uploaded','digest':'sha256:'+release.digest(path)})
                        if len(uploaded)==2 and not interrupted:
                            interrupted=True
                            raise subprocess.CalledProcessError(1,args)
                return subprocess.CompletedProcess(args,0)
            with patch.object(release,'ROOT',root),patch.object(release.subprocess,'run',command),patch.object(release.subprocess,'check_output',return_value='release\n'):
                with self.assertRaises(subprocess.CalledProcessError):
                    release.publish(api,'0.8.0','release',directory)
                api.release['body']='maintainer edits'
                result=release.publish(api,'0.8.0','release',directory)
                self.assertTrue(result['draft'])
                self.assertEqual(result['body'],'maintainer edits')
                self.assertEqual(len(uploaded),8)
                self.assertEqual(api.tag,'release')
                self.assertEqual(release.publish(api,'0.8.0','release',directory),result)
                api.release['draft']=False
                with self.assertRaises(ValueError):
                    release.publish(api,'0.8.0','release',directory)

    @unittest.skipUnless(platform.system() == 'Darwin', 'Mach-O relocation requires macOS')
    def test_macos_recursive_library_relocation_runs_without_original_libraries(self):
        import package
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); original=root/'original'; original.mkdir()
            stage=root/'stage'; (stage/'MacOS').mkdir(parents=True)
            (original/'b.c').write_text('int b(void) { return 42; }\n')
            (original/'a.c').write_text('extern int b(void); int a(void) { return b(); }\n')
            (original/'main.c').write_text('extern int a(void); int main(void) { return a() != 42; }\n')
            env=dict(os.environ,MACOSX_DEPLOYMENT_TARGET='15.0')
            def clang(*args):
                subprocess.run(['clang','-Wl,-headerpad_max_install_names',*map(str,args)],env=env,check=True,capture_output=True)
            clang('-dynamiclib',original/'b.c','-o',original/'libb.dylib','-Wl,-install_name,'+str(original/'libb.dylib'))
            clang('-dynamiclib',original/'a.c','-o',original/'liba.dylib','-L'+str(original),'-lb','-Wl,-install_name,'+str(original/'liba.dylib'))
            clang(original/'main.c','-o',original/'main','-L'+str(original),'-la')
            binary=stage/'MacOS/main';shutil.copy2(original/'main',binary)
            libraries=package.stage_macos(original/'main',binary,stage)
            self.assertEqual({p.name for p in libraries},{'liba.dylib','libb.dylib'})
            shutil.rmtree(original)
            for path in [*libraries,binary]:
                subprocess.run(['codesign','--force','--sign','-',str(path)],check=True,capture_output=True)
            subprocess.run([str(binary)],check=True)

    def test_versions(self):
        for version in ("0.8.0", "1.2.3-rc.1", "0.0.0-alpha"):
            release.version_parts(version)
        for version in ("v1.0.0", "1.0", "01.0.0", "1.0.0-01", "1.0.0+meta", "65536.0.0", "1.0.0\n", "1٢.0.0", "$(evil)", "../x"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.version_parts(version)

    def test_lockfile_only_changes_own_package(self):
        result = release.bump_files(MANIFEST, LOCK, "0.8.0-rc.1")
        self.assertEqual(result["Cargo.toml"], MANIFEST.replace('version = "0.7.0"', 'version = "0.8.0-rc.1"'))
        self.assertEqual(result["Cargo.lock"], LOCK.replace('version = "0.7.0"', 'version = "0.8.0-rc.1"'))

    def test_mismatched_lock_rejected(self):
        with self.assertRaises(ValueError):
            release.bump_files(MANIFEST, LOCK.replace('version = "0.7.0"', 'version = "0.6.0"'), "0.8.0")

    def test_unchanged_version_rejected_before_creating_an_empty_pr(self):
        api = FakeGitHub()
        with self.assertRaises(ValueError):
            release.prepare(api, "0.7.0", "source", "123")
        self.assertIsNone(api.branch)

    def test_prepare_and_retry_same_run(self):
        api = FakeGitHub()
        self.assertEqual(release.prepare(api, "0.8.0", "source", "123"), "release")
        self.assertEqual(release.prepare(api, "0.8.0", "source", "123"), "release")

    def test_other_run_cannot_take_over_branch(self):
        api = FakeGitHub(); release.prepare(api, "0.8.0", "source", "123")
        with self.assertRaises(ValueError):
            release.prepare(api, "0.8.0", "source", "124")

    def test_branch_changed_after_preparation(self):
        api = FakeGitHub(); release.prepare(api, "0.8.0", "source", "123")
        api.files["release"]["Cargo.toml"] += '\n# unexpected change\n'
        with self.assertRaises(ValueError):
            release.prepare(api, "0.8.0", "source", "123")

    def test_unverified_commit_rejected(self):
        api = FakeGitHub(); release.prepare(api, "0.8.0", "source", "123")
        api.commit["commit"]["verification"]["verified"] = False
        with self.assertRaises(ValueError):
            release.prepare(api, "0.8.0", "source", "123")

    def test_published_release_rejected_before_branch_write(self):
        api = FakeGitHub(); api.release = {"draft": False}
        with self.assertRaises(ValueError):
            release.prepare(api, "0.8.0", "source", "123")
        self.assertIsNone(api.branch)

    def test_conflicting_tag_rejected(self):
        api = FakeGitHub(); release.prepare(api, "0.8.0", "source", "123")
        api.tag = "different"
        with self.assertRaises(ValueError):
            release.prepare(api, "0.8.0", "source", "123")

    def test_upload_retry_preserves_complete_files_and_removes_only_empty_starter(self):
        api = FakeGitHub(); draft = {"id": 1, "draft": True}
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            for name in ("one", "two", "three"):
                (directory/name).write_text(name)
            api.assets = [{"id": 1, "name": "one", "state": "uploaded", "size": 3, "digest": "sha256:"+release.digest(directory/'one')},
                          {"id": 2, "name": "two", "state": "starter", "size": 0}]
            self.assertEqual(release.missing_assets(api,draft,directory,["one","two","three"]), ["three","two"])
            self.assertEqual([a['name'] for a in api.assets], ['one'])
            (directory/'one').write_text('modified')
            with self.assertRaises(ValueError):
                release.missing_assets(api,draft,directory,["one","two","three"])

    def test_complete_matrix_and_tampering(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            for target in release.TARGETS:
                assets={}
                for name in release.asset_names("0.8.0",target):
                    (directory/name).write_bytes(b'package fixture')
                    assets[name]={"sha256":release.digest(directory/name),"size":15}
                (directory/f'{target}.json').write_text(json.dumps({"version":"0.8.0","commit":"abc","target":target,"assets":assets}))
            self.assertEqual(len(release.validate_assets(directory,"0.8.0","abc")),6)
            with self.assertRaises(ValueError):
                release.validate_assets(directory,"0.8.0","different")
            (directory/release.asset_names("0.8.0")[0]).write_bytes(b'tampered')
            with self.assertRaises(ValueError):
                release.validate_assets(directory,"0.8.0","abc")


if __name__ == '__main__':
    unittest.main(verbosity=2)
