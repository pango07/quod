#!/usr/bin/env python3
"""Audit pinned completed native VM evidence as data, never as executable code.

The archive size/SHA and runtime/reader source hashes below are independent
trusted pins from run42411734417. --trusted-source must refer to a trusted
checkout (or trusted git-object export) with those exact files, not artifact
contents. The child uses isolated Python without site initialization.
"""
import argparse,hashlib,json,pathlib,stat,subprocess,sys,unicodedata,zipfile
sys.dont_write_bytecode=True
SOURCE_PIN='1ac7bab428835914dd14c6f5907ce5c02dfe3445'
ARTIFACT_ID='13590591522'
ARTIFACT_BYTES=187402743
ARTIFACT_SHA256='554c38e5590ca2a840bb92927fc30aa68bba4ed224e52ea72d54af8eebb7f5d4'
PINS={'build.py':'0d1ef0f8a12e88721eb99e1086fdeebb346b0bf7cac4cf19803e72c59ea9956f','guest-init.c':'c470b31f935661dd72ed75404c8c484cc80db0cb057aa355e185e9b928cc1dcb','guest-negative.c':'3776c8da8365e59c1f001d0ce49081df60fbbd289abcee46c5ea4d9438e84420','probe.py':'854a5f5db81b77af39fd5eb625f805e2ceeed72dfac34d6619e1dbce5fe524cd','audit.py':'22a474702cc5f046d984fc233b69c54d4545111113cd96256858bde905df6cae'}
def require(c,m):
    if not c:raise ValueError(m)
def sha(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
    return h.hexdigest()
def canonical(v):return json.dumps(v,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode()
ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('root',type=pathlib.Path)
ap.add_argument('output',type=pathlib.Path)
ap.add_argument('--trusted-source',type=pathlib.Path,default=pathlib.Path(__file__).resolve().parent,
                help='Trusted source checkout containing the five exact pinned runtime/reader files; never an artifact directory')
args=ap.parse_args()
ROOT=args.root;OUTPUT=args.output;artifact_id,size,digest=ARTIFACT_ID,ARTIFACT_BYTES,ARTIFACT_SHA256
require(ROOT.is_dir() and not ROOT.is_symlink(),'Missing/linked artifact root')
ROOT=ROOT.resolve();OUTPUT=OUTPUT.resolve();SOURCE=args.trusted_source.resolve()
ZIP=pathlib.Path(str(ROOT)+'.zip')
require(not OUTPUT.is_relative_to(ROOT),'Output lies in immutable artifact')
require(OUTPUT!=ZIP.resolve() and OUTPUT not in {SOURCE/name for name in PINS} and
        OUTPUT!=pathlib.Path(__file__).resolve(),'Output would overwrite trusted source or the immutable ZIP')
require(SOURCE.is_dir() and not SOURCE.is_relative_to(ROOT) and not ROOT.is_relative_to(SOURCE),
        'Trusted reader/runtime sources must be separate from downloaded artifact data')
report={'schema':'quod-native-vm-bound-artifact-audit-v1','run':42411734417,'sourcePin':'1ac7bab428835914dd14c6f5907ce5c02dfe3445','artifactCodeExecuted':False,'receiptAdmission':False,'ok':False,'issues':[]}
try:
    require(0<size<=512*1024**2 and ZIP.is_file() and not ZIP.is_symlink() and
            ZIP.stat().st_size==size and len(digest)==64 and digest==sha(ZIP),'Pinned archive size/SHA differs')
    report['archive']={'artifactId':artifact_id,'bytes':size,'sha256':digest,'verifiedFromRawZip':True}
    for name,h in PINS.items():
        source=SOURCE/name
        require(source.is_file() and not source.is_symlink() and source.stat().st_size<=1024*1024 and sha(source)==h,
                'Frozen trusted regular reader/runtime source differs: '+name)
    require(not ROOT.is_symlink(),'Extracted root is a symlink')
    expected_files,expected_dirs,folded=set(),set(),set()
    with zipfile.ZipFile(pathlib.Path(str(ROOT)+'.zip')) as zipped:
        entries=zipped.infolist()
        require(0<len(entries)<=150000 and sum(e.file_size for e in entries)<=1536*1024**2,'ZIP inventory bound')
        names=set()
        for entry in entries:
            name=pathlib.PurePosixPath(entry.filename)
            require(name.parts and not name.is_absolute() and '..' not in name.parts and '\\' not in entry.filename and
                    str(name)==entry.filename.rstrip('/') and not any(':' in p or '\x00' in p for p in name.parts),'Noncanonical ZIP path')
            normalized=unicodedata.normalize('NFC',str(name)).casefold()
            require(str(name) not in names and normalized not in folded and not entry.flag_bits&1,'Duplicate/aliased/encrypted ZIP entry')
            names.add(str(name));folded.add(normalized)
            mode=stat.S_IFMT(entry.external_attr>>16)
            require(mode in {0,stat.S_IFREG,stat.S_IFDIR} and 0<=entry.file_size<=512*1024**2,'ZIP type/size bound')
            target=ROOT.joinpath(*name.parts)
            require(not target.is_symlink(),'Extracted artifact contains a symlink')
            for parent in name.parents:
                if str(parent)!='.':expected_dirs.add(str(parent))
            if entry.is_dir():
                require(mode in {0,stat.S_IFDIR} and target.is_dir(),'ZIP directory type differs')
                expected_dirs.add(str(name));continue
            require(mode in {0,stat.S_IFREG} and target.is_file() and target.stat().st_size==entry.file_size,'Extracted file missing/size differs')
            expected_files.add(str(name));h=hashlib.sha256();count=0
            with zipped.open(entry) as raw:
                for chunk in iter(lambda:raw.read(1024*1024),b''):
                    count+=len(chunk);require(count<=entry.file_size,'ZIP payload exceeds bound');h.update(chunk)
            require(count==entry.file_size and h.hexdigest()==sha(target),'Extracted file is not the pinned ZIP payload: '+str(name))
        actual_files,actual_dirs=set(),set()
        for index,path in enumerate(ROOT.rglob('*')):
            require(index<150000 and not path.is_symlink(),'Extracted inventory bound/link')
            info=path.lstat();relative=path.relative_to(ROOT).as_posix()
            if stat.S_ISDIR(info.st_mode):actual_dirs.add(relative)
            else:
                require(stat.S_ISREG(info.st_mode),'Extracted special file');actual_files.add(relative)
        require(actual_files==expected_files and actual_dirs==expected_dirs,'Whole extracted inventory differs from pinned ZIP')
    report['zipBinding']={'complete':True,'files':len(expected_files),'directories':len(expected_dirs),
                          'entries':len(entries),'uncompressedBytes':sum(e.file_size for e in entries),
                          'pathsSha256':hashlib.sha256(canonical(sorted(expected_files))).hexdigest()}
    completed=subprocess.run([sys.executable,'-I','-S',str(SOURCE/'audit.py'),str(ROOT/'native-vm-image'),str(ROOT/'native-vm-evidence')],stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=False,timeout=90,env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8','TZ':'UTC','PYTHONDONTWRITEBYTECODE':'1'})
    require(len(completed.stdout)<=16*1024**2 and not completed.stderr,'Trusted data reader output/error bound')
    audit=json.loads(completed.stdout)
    report['independentAudit']=audit;report['readerExitCode']=completed.returncode;report['sourceSha256']=PINS
    report['childPythonIsolation']=['-I','-S']
    require(audit==json.loads((ROOT/'native-vm-audit.json').read_bytes()),'Remote and local independent reader results differ')
    for phase in ['boundary','background','build-1','build-2','release-1','release-2']:
        response=json.loads((ROOT/'native-vm-evidence'/phase/'response.json').read_bytes())
        for row in response['effects']['dist']:
            require(all(key in row for key in ['inode','device','links','mtimeSeconds','mtimeNanoseconds','atimeSeconds','atimeNanoseconds','ctimeSeconds','ctimeNanoseconds']),'Full output metadata omitted')
            require(all(type(row[key]) is int and 0<=row[key]<1000000000 for key in ['mtimeNanoseconds','atimeNanoseconds','ctimeNanoseconds']),'Output timestamp fraction invalid')
    for name,h in PINS.items():require(sha(SOURCE/name)==h,'Trusted source changed during audit: '+name)
    require((completed.returncode==0) is (audit['ok'] is True),'Reader exit/result differs')
    report['files']={p.relative_to(ROOT).as_posix():{'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(ROOT.rglob('*')) if p.is_file()}
    report['ok']=audit['ok'] is True
except Exception as exc:report['issues'].append(str(exc))
OUTPUT.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'ok':report['ok'],'issues':report['issues'],'output':str(OUTPUT),'sha256':sha(OUTPUT)}))
raise SystemExit(0 if report['ok'] else 1)
