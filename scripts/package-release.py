#!/usr/bin/env python3
"""Build one self-contained source + prebuilt-binaries ZIP from verified builds.

python scripts/package-release.py --windows-bin target/x86_64-pc-windows-gnu/release \
  --linux-bin target/release --output dist/IronSetup-v0.1.0-portable.zip
"""
from __future__ import annotations
import argparse,hashlib,json,os,shutil,subprocess,tempfile,zipfile
from pathlib import Path

def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--windows-bin',required=True,type=Path)
    ap.add_argument('--linux-bin',required=True,type=Path)
    ap.add_argument('--third-party',type=Path,default=Path('third-party'))
    ap.add_argument('--output',type=Path,default=Path('dist/IronSetup-v0.1.0-portable.zip'))
    ns=ap.parse_args();root=Path(__file__).resolve().parent.parent
    output=ns.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
    windows=ns.windows_bin.resolve();linux=ns.linux_bin.resolve()
    notices=ns.third_party.resolve()
    if not (notices/'dependencies.json').is_file(): raise SystemExit('Run scripts/collect-licenses.py before packaging.')
    with tempfile.TemporaryDirectory(prefix='ironsetup-release-',dir=output.parent) as temporary:
        stage=Path(temporary)/'IronSetup-v0.1.0';stage.mkdir()
        names=['AGENTS.md','Cargo.toml','Cargo.lock','.gitignore','LICENSE','README.md','README.zh-TW.md','THIRD_PARTY_NOTICES.md','Start-IronSetup.cmd','Build-Demo.cmd']
        for name in names: shutil.copy2(root/name,stage/name)
        allowed={'.rs','.toml','.lock','.md','.json','.iss','.txt','.ps1','.sh','.py','.log','.cmd'}
        for directory in ['crates','docs','examples','scripts','evidence']:
            for source in sorted((root/directory).rglob('*')):
                relative=source.relative_to(root)
                if not source.is_file() or source.is_symlink() or source.suffix not in allowed: continue
                if '__pycache__' in relative.parts or (len(relative.parts)>1 and relative.parts[:2]==('evidence','tmp')): continue
                target=stage/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,target)
        wbin=stage/'bin/windows-x64';lbin=stage/'bin/linux-x64';wbin.mkdir(parents=True);lbin.mkdir(parents=True)
        for name in ['ironsetup','ironsetup-stub','ironsetup-studio']:
            source=windows/(name+'.exe')
            if not source.is_file(): raise SystemExit(f'Missing verified binary: {source}')
            shutil.copy2(source,wbin/source.name)
        for name in ['ironsetup','ironsetup-stub']:
            shutil.copy2(linux/name,lbin/name);(lbin/name).chmod(0o755)
        demo=stage/'examples/payload/windows/IronSetup-Demo.exe';demo.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(windows/'ironsetup-demo.exe',demo)
        shutil.copytree(notices,stage/'third-party')
        host=(wbin/'ironsetup.exe') if os.name=='nt' else (lbin/'ironsetup')
        samples=stage/'samples';samples.mkdir()
        builds=[]
        def compile_sample(script,output_path,stub=None):
            argv=[str(host),'compile',str(stage/script),'--output',str(output_path),'--json']
            argv+=['--stub',str(stub)] if stub else ['--package-only']
            p=subprocess.run(argv,check=True,text=True,encoding='utf-8',capture_output=True)
            report=json.loads(p.stdout);report['output']=str(output_path.relative_to(stage));report['stderr']=p.stderr;builds.append(report)
        compile_sample('examples/windows-demo.iss',samples/'IronSetup-Demo-Setup.exe',wbin/'ironsetup-stub.exe')
        compile_sample('examples/hello.iss',samples/'IronSetup-Hello.ispkg')
        compile_sample('examples/hello.iss',samples/'IronSetup-Hello.run',lbin/'ironsetup-stub')
        (stage/'evidence/sample-builds.json').write_text(json.dumps(builds,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        rev=subprocess.run(['git','rev-parse','HEAD'],cwd=root,text=True,capture_output=True)
        (stage/'evidence/package-build.json').write_text(json.dumps({'version':'0.1.0','source_revision':rev.stdout.strip() if rev.returncode==0 else None,'binaries':{str(p.relative_to(stage)):digest(p) for p in sorted((stage/'bin').rglob('*')) if p.is_file()},'windows_native_execution':'NOT_RUN; see VERIFICATION.md'},indent=2)+'\n',encoding='utf-8')
        files=sorted(p for p in stage.rglob('*') if p.is_file())
        (stage/'SHA256SUMS.txt').write_text(''.join(f'{digest(p)}  {p.relative_to(stage).as_posix()}\n' for p in files),encoding='utf-8')
        zip_temp=Path(temporary)/output.name
        with zipfile.ZipFile(zip_temp,'w',compression=zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
            for path in sorted(p for p in stage.rglob('*') if p.is_file()):
                info=zipfile.ZipInfo('IronSetup-v0.1.0/'+path.relative_to(stage).as_posix(),(2026,10,25,0,0,0))
                info.compress_type=zipfile.ZIP_DEFLATED;info.create_system=3;info.external_attr=(path.stat().st_mode&0xffff)<<16
                archive.writestr(info,path.read_bytes(),compresslevel=9)
        with zipfile.ZipFile(zip_temp) as archive:
            damaged=archive.testzip()
            if damaged: raise RuntimeError(f'ZIP verification failed: {damaged}')
            count=len(archive.infolist())
        os.replace(zip_temp,output)
    print(json.dumps({'file':str(output),'size_bytes':output.stat().st_size,'sha256':digest(output),'zip_entries':count},indent=2))
if __name__=='__main__': main()
