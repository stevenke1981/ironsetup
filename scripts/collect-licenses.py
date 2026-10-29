#!/usr/bin/env python3
"""Collect Cargo package and Rust toolchain notices for portable redistribution."""
import argparse,json,shutil,subprocess
from pathlib import Path

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--output',type=Path,default=Path('third-party'))
    ap.add_argument('--mingw-prefix',type=Path)
    ns=ap.parse_args()
    root=Path(__file__).resolve().parent.parent
    result=subprocess.run(['cargo','metadata','--offline','--locked','--format-version','1'],cwd=root,check=True,capture_output=True,text=True)
    metadata=json.loads(result.stdout)
    destination=ns.output.resolve();destination.mkdir(parents=True,exist_ok=True)
    packages=[]
    for package in sorted(metadata['packages'],key=lambda p:(p['name'],p['version'])):
        if package.get('source') is None: continue
        source=Path(package['manifest_path']).parent
        out=destination/'licenses'/(package['name']+'-'+package['version'])
        copied=[]
        for path in sorted(source.iterdir()):
            if path.is_file() and path.name.lower().startswith(('license','licence','copying','notice','copyright')):
                out.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,out/path.name);copied.append(path.name)
        if not copied:
            # Some target-only packages publish their grant in README/AUTHORS.
            for name in ['README.md','AUTHORS']:
                path=source/name
                if path.is_file():
                    out.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,out/name);copied.append(name)
        packages.append({'name':package['name'],'version':package['version'],'license':package.get('license'),'repository':package.get('repository'),'notice_files':copied})
    (destination/'dependencies.json').write_text(json.dumps(packages,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    sysroot=Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip())
    rust_docs=sysroot/'share/doc/rust'
    toolchain=destination/'toolchain';toolchain.mkdir(exist_ok=True)
    if (rust_docs/'COPYRIGHT-library.html').is_file(): shutil.copyfile(rust_docs/'COPYRIGHT-library.html',toolchain/'Rust-COPYRIGHT-library.html')
    if (rust_docs/'licenses').is_dir(): shutil.copytree(rust_docs/'licenses',toolchain/'licenses',dirs_exist_ok=True)
    if ns.mingw_prefix:
        for package in ['mingw-w64-common','mingw-w64-x86-64-dev','gcc-mingw-w64-base']:
            source=ns.mingw_prefix/'usr/share/doc'/package/'copyright'
            if source.is_file(): shutil.copyfile(source,toolchain/(package+'-copyright.txt'))
        gpl=Path('/usr/share/common-licenses/GPL-3')
        if gpl.is_file(): shutil.copyfile(gpl,toolchain/'GPL-3.txt')
    print(json.dumps({'packages':len(packages),'files':sum(1 for p in destination.rglob('*') if p.is_file()),'output':str(destination)}))
if __name__=='__main__': main()
