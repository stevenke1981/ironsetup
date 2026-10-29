#!/usr/bin/env python3
"""Real CLI/SFX lifecycle acceptance. Uses only disposable directories.

python scripts/smoke.py --bin-dir target/release --report evidence/smoke-linux.json
python scripts/smoke.py --bin-dir target/x86_64-pc-windows-gnu/release --runner /usr/lib/wine/wine64 --report evidence/smoke-wine.json
"""
from __future__ import annotations
import argparse, hashlib, json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

def main() -> int:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--bin-dir',type=Path,required=True)
    ap.add_argument('--runner',type=Path)
    ap.add_argument('--report',type=Path)
    ns=ap.parse_args()
    root=Path(__file__).resolve().parent.parent
    win=bool(ns.runner) or os.name=='nt'
    suffix='.exe' if win else ''
    compiler=ns.bin_dir.resolve()/('ironsetup'+suffix)
    stub=ns.bin_dir.resolve()/('ironsetup-stub'+suffix)
    for p in [compiler,stub]:
        if not p.is_file(): raise SystemExit(f'Missing binary: {p}')
    checks=[]
    def arg(value):
        if isinstance(value,Path):
            text=str(value.resolve())
            return ('Z:'+text.replace('/','\\')) if ns.runner else text
        return str(value)
    def run(name,arguments,expected=0,executable=compiler):
        command=([str(ns.runner)] if ns.runner else [])+[str(executable)]+[arg(x) for x in arguments]
        p=subprocess.run(command,cwd=root,text=True,encoding='utf-8',errors='replace',capture_output=True,timeout=120)
        checks.append({'name':name,'argv':command,'expected_exit':expected,'exit_code':p.returncode,'result':'PASS' if p.returncode==expected else 'FAIL','stdout':p.stdout,'stderr':p.stderr})
        if p.returncode!=expected: raise AssertionError(f'{name}: exit {p.returncode}, wanted {expected}\n{p.stdout}\n{p.stderr}')
        return p
    def check(name,condition):
        checks.append({'name':name,'result':'PASS' if condition else 'FAIL'})
        if not condition: raise AssertionError(name)
    report={'platform':'Windows EXE on Wine' if ns.runner else sys.platform,'native_windows':os.name=='nt','binary_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [compiler,stub]},'checks':checks}
    try:
        with tempfile.TemporaryDirectory(prefix='IronSetup-smoke-') as tmp:
            t=Path(tmp);pkg=t/'hello.ispkg';pkg2=t/'hello-again.ispkg';target=t/'Installed App'
            run('script check',['check',root/'examples/hello.iss'])
            run('unsupported Pascal fails closed',['check',root/'examples/unsupported-pascal.iss'],2)
            run('compile verified archive',['compile',root/'examples/hello.iss','--package-only','--output',pkg])
            run('repeat deterministic compile',['compile',root/'examples/hello.iss','--package-only','--output',pkg2])
            check('compiled bytes deterministic',pkg.read_bytes()==pkg2.read_bytes())
            info=json.loads(run('inspect archive',['inspect',pkg]).stdout)
            check('manifest has all three payload files',len(info['files'])==3)
            run('install explicit component subset',['install',pkg,'--dir',target,'--components','core','--tasks=','--no-run','--log',t/'install.log'])
            check('selected files installed; docs omitted',(target/'readme.txt').is_file() and not (target/'docs').exists())
            run('verify clean installation',['verify','--dir',target])
            (target/'my-notes.txt').write_text('User-owned file',encoding='utf-8')
            (target/'readme.txt').write_text('User-modified content',encoding='utf-8')
            run('modified file causes exit 3',['verify','--dir',target],3)
            run('repeat install refuses existing journal',['install',pkg,'--dir',target,'--no-run'],2)
            check('refused reinstall made no writes',(target/'readme.txt').read_text()=='User-modified content')
            run('uninstall',['uninstall','--dir',target])
            check('unowned and modified files preserved',(target/'my-notes.txt').read_text()=='User-owned file' and (target/'readme.txt').read_text()=='User-modified content')
            check('unmodified managed config removed',not (target/'config/settings.toml').exists())
            restore=t/'Restore Original';restore.mkdir();(restore/'readme.txt').write_bytes(b'Original bytes')
            if not win: (restore/'readme.txt').chmod(0o600)
            run('install over existing file',['install',pkg,'--dir',restore,'--no-run'])
            run('uninstall restores existing file',['uninstall','--dir',restore])
            check('original bytes restored',(restore/'readme.txt').read_bytes()==b'Original bytes')
            if not win: check('original permissions restored',(restore/'readme.txt').stat().st_mode&0o777==0o600)
            corrupt=t/'corrupt.ispkg';bad=bytearray(pkg.read_bytes());bad[len(bad)//2]^=0x80;corrupt.write_bytes(bad)
            run('tampered package rejected',['install',corrupt,'--dir',t/'Must Not Exist'],2)
            check('integrity failure creates no install directory',not (t/'Must Not Exist').exists())
            sfx=t/('Self Extracting Setup.exe' if win else 'self-extracting.run');sfx_target=t/'SFX Install'
            run('compile executable installer',['compile',root/'examples/hello.iss','--stub',stub,'--output',sfx])
            run('execute self-extracting installer',['/VERYSILENT','--dir',sfx_target,'/NORESTART','--no-run'],executable=sfx)
            run('verify SFX installation',['verify','--dir',sfx_target])
            run('run standalone installed uninstaller',['--silent','--log',t/'uninstall-sfx.log'],executable=sfx_target/'unins000.exe')
            deadline=time.monotonic()+20
            while sfx_target.exists() and time.monotonic()<deadline:
                time.sleep(0.1)
            check('clean SFX installation fully removed',not sfx_target.exists())
        report['result']='PASS'
    except Exception as error:
        report['result']='FAIL';report['error']=str(error)
        print(str(error),file=sys.stderr)
    finally:
        if ns.report:
            ns.report.parent.mkdir(parents=True,exist_ok=True)
            ns.report.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f"{report['result']}: {len(checks)} checks ({report['platform']})")
    return 0 if report['result']=='PASS' else 1

if __name__=='__main__': raise SystemExit(main())
