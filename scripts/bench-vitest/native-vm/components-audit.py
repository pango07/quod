#!/usr/bin/env python3
"""Independent retained-data component audit. Never executes artifact code."""
import base64, hashlib, importlib.util, json, pathlib, re, stat, sys, unicodedata, zipfile
sys.dont_write_bytecode = True

SOURCE = pathlib.Path(__file__).resolve().parent
if len(sys.argv) != 3:
    raise SystemExit('usage: components-audit.py VERIFIED_ARTIFACT_ROOT OUTPUT_JSON')
ROOT = pathlib.Path(sys.argv[1]).resolve()
if pathlib.Path(sys.argv[1]).is_symlink():
    raise SystemExit('Extracted artifact root must not be a symlink')
OUTPUT = pathlib.Path(sys.argv[2]).resolve()
if OUTPUT.is_relative_to(ROOT):
    raise SystemExit('Audit output must be outside the immutable extracted artifact root')
SOURCE_HASHES = {"build.py":"0d1ef0f8a12e88721eb99e1086fdeebb346b0bf7cac4cf19803e72c59ea9956f","guest-init.c":"8506d5e9c7148a71c39eded7305e95fc361955c8272791f657864df4a527968e","guest-negative.c":"3776c8da8365e59c1f001d0ce49081df60fbbd289abcee46c5ea4d9438e84420","probe.py":"6f7b0246a96d5bcda64179855527937400f3ddbd0be05562a271ed8d4256addd"}
IMAGE, EVIDENCE = ROOT/'native-vm-image', ROOT/'native-vm-evidence'
def sha(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
    return h.hexdigest()
def read(p):return json.loads(p.read_bytes())
def canonical(v):return json.dumps(v,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode()
def decode(v):return base64.b64decode(v,validate=True)
def require(c,m):
    if not c:raise ValueError(m)

helper_path=SOURCE/'audit.py';helper_sha=sha(helper_path)
spec=importlib.util.spec_from_file_location('trusted_native_vm_audit',helper_path)
helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
report={'schema':'quod-native-vm-components-v1','run':42406327547,
        'sourcePin':'7108de81d1f3a12063bc797cf73aa5e8bdfb8c38','artifactCodeExecuted':False,
        'fullInvestigationPassed':False,'receiptAdmission':False,'issues':[]}
try:
    transport=read(pathlib.Path(str(ROOT)+'-verified.json'))
    require(pathlib.Path(str(ROOT)+'.zip').stat().st_size==185051079 and transport['bytes']==185051079 and
            transport['sha256']=='30e070ca9a3043458d072cbf982f131cb9788b2d4e79a78c2ee436b51fae5895' and
            sha(pathlib.Path(str(ROOT)+'.zip'))==transport['sha256'],'Archive authority/digest changed')
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
    metadata=read(IMAGE/'image.json');policy=read(EVIDENCE/'policy.json');summary=read(EVIDENCE/'result.json')
    require((IMAGE/'image.json').read_bytes()==(EVIDENCE/'image.json').read_bytes(),'Image identity copies differ')
    require(metadata['source']=='5eaa7e5160736054d53f8b71e2c522835eb427e0' and metadata['receiptAdmission'] is False,'Wrong source/scope')
    for name,record in [('kernel',metadata['kernel']),('initramfs.cpio.gz',metadata['initramfs'])]:
        require(sha(IMAGE/name)==record['sha256'] and (IMAGE/name).stat().st_size==record['bytes'],'Image blob differs')
    for name,field in [('build.py','builderSha256'),('guest-init.c','guestInitSourceSha256'),('guest-negative.c','negativeSourceSha256')]:
        require(SOURCE_HASHES[name]==metadata[field],'Pinned trusted image source changed: '+name)
    require(policy['probeSha256']==SOURCE_HASHES['probe.py'],'Pinned trusted supervisor source changed')
    require(hashlib.sha256(canonical(metadata['inventory'])).hexdigest()==metadata['inventorySha256'],'Inventory digest differs')
    archive=helper.audit_archive(IMAGE/'initramfs.cpio.gz',metadata['inventory'])
    require(archive=={'entries':32260,'storedDataBytes':499114944},'Unexpected actual image inventory')
    require(policy['cpu']=='Haswell-noTSX,rdrand=off,rdseed=off,rdtscp=off,pmu=off' and
            policy['vcpus']==1 and policy['memoryMiB']==4096 and policy['tcgThread']=='single' and
            policy['icount']=='shift=0,align=off,sleep=off' and policy['seed']==42 and
            policy['rtc']=='base=2023-11-14T22:13:20,clock=vm,driftfix=none' and
            policy['uuid']=='3a84431b-2034-4c41-8f61-157200000001' and
            policy['hostMounts']==policy['blockDevices']==policy['inputChannels']==[] and policy['receiptAdmission'] is False,'Wrong enforced VM policy')
    require(policy['hostEnvironment']=={'PATH':'/usr/bin:/bin','TZ':'UTC','LANG':'C.UTF-8'} and policy['watchdogSeconds']==180,'Wrong environment/watchdog')
    key=policy['executionClassSha256'];identity={k:v for k,v in policy.items() if k!='executionClassSha256'}
    require(key==hashlib.sha256(b'quod-native-vm-class-v1\n'+canonical(identity)).hexdigest(),'Class key differs')
    require(summary['ok'] is False and summary['failure']=='QEMU watchdog expired; completion is incomplete' and
            [r['name'] for r in summary['runs']]==['boundary','background','build-1','build-2','release-1'],'Partial run state differs')
    invocations=[];phases=[];data={}
    for name in ['boundary','background','build-1','build-2','release-1']:
        case=EVIDENCE/name;command=name.split('-')[0];result=read(case/'result.json');invocation=read(case/'invocation.json')
        require(result==next(r for r in summary['runs'] if r['name']==name),'Summary and phase result differ')
        argv=invocation['argv'];remote_image=str(pathlib.PurePosixPath(argv[argv.index('-kernel')+1]).parent)
        serial_arg=argv[argv.index('-chardev')+1]
        expected=[metadata['qemu']['path'],'-no-user-config','-nodefaults','-display','none','-monitor','none','-nic','none',
                  '-accel','tcg,thread=single','-machine',policy['machine'],'-cpu',policy['cpu'],'-smp','1','-m','4096',
                  '-uuid',policy['uuid'],'-seed','42','-icount',policy['icount'],'-rtc',policy['rtc'],'-L',metadata['qemu']['dataDirectory'],
                  '-bios',metadata['qemu']['bios'],'-kernel',remote_image+'/kernel','-initrd',remote_image+'/initramfs.cpio.gz',
                  '-append','init=/init rdinit=/init console=ttyS0,115200n8 quiet loglevel=0 nokaslr random.trust_cpu=off random.trust_bootloader=off panic=-1 oops=panic quod.command='+command,
                  '-chardev',serial_arg,'-device','isa-serial,chardev=trusted-output,index=0','-no-reboot']
        require(argv==expected and serial_arg.startswith('file,id=trusted-output,path=') and serial_arg.endswith('/'+name+'/serial.raw'),'Actual QEMU invocation differs')
        require(invocation['environment']==policy['hostEnvironment'] and invocation['stdin']=='DEVNULL' and
                invocation['watchdogSeconds']==(60 if command in {'boundary','background'} else 180) and
                invocation['policySha256']==sha(EVIDENCE/'policy.json') and invocation['imageMetadataSha256']==sha(IMAGE/'image.json'),'Invocation policy identity differs')
        invocations.append(invocation)
        if name=='release-1':
            require(result['ok'] is False and result['timedOut'] is True and result['qemuExitCode']==-9 and
                    180<=result['seconds']<181 and not (case/'response.json').exists(),'Release timeout was misreported')
            report['release']={'completed':False,'timedOut':True,'hostSeconds':result['seconds'],
                               'serialBytes':(case/'serial.raw').stat().st_size,'capturedProgressAvailable':False,
                               'reason':'Child output stays in guest capture pipes until final quiescence; no progress envelope was emitted'}
            continue
        require(result['ok'] is True and result['timedOut'] is False and result['qemuExitCode']==0,'Completed QEMU phase did not succeed')
        serial=(case/'serial.raw').read_bytes();matches=list(re.finditer(rb'(?:^|\n)QUOD_NATIVE_VM_RESULT_V1 ([0-9]+) ([0-9a-f]{64})\n',serial))
        require(len(matches)==1,'Not exactly one trusted final envelope')
        match=matches[0];size=int(match[1]);body=serial[match.end():match.end()+size]
        require(len(body)==size and hashlib.sha256(body).hexdigest().encode()==match[2] and
                serial[match.end()+size:].startswith(b'\nQUOD_NATIVE_VM_END\n') and body==(case/'response.json').read_bytes() and
                sha(case/'response.json')==result['responseSha256'] and sha(case/'serial.raw')==result['serialSha256'],'Raw response/envelope differs')
        response=json.loads(body);proof=response['proof'];effects=response['effects'];data[name]=response
        require(response['command']==command and response['receiptAdmission'] is False and response['exitCode']==response['signal']==0,'Wrong native command response')
        require(all(proof[k] is True for k in ['entropySeededBeforeChild','loopbackEnabled','sourceReadOnly','captureComplete']) and
                proof['remainingProcesses']==0 and len(decode(proof['entropyProbeBase64']))==32,'Boundary/quiescence incomplete')
        if name=='background':
            require(response['ok'] is False and response['complete'] is False and response['leftoversDetected']>0 and response['killedProcesses']>0 and
                    'processes remained' in response['failure'],'Background descendant was not rejected')
        else:
            require(response['ok'] is True and response['complete'] is True and response['leftoversDetected']==response['killedProcesses']==0 and
                    response['failure']=='','Positive command was not complete')
        for field,file in [('stdoutBase64','command.stdout'),('stderrBase64','command.stderr')]:
            require(decode(effects[field])==(case/file).read_bytes(),'Actual stdout/stderr bytes differ')
        require(hashlib.sha256(canonical(effects)).hexdigest()==result['effectsSha256'],'Actual effect digest differs')
        paths=[];size_sum=0
        for row in effects['dist']:
            path=decode(row['pathBase64']);paths.append(path)
            require(not path.startswith(b'/') and b'..' not in path.split(b'/') and row['uid']==row['gid']==1000 and
                    0<=row['mode']<=0o7777 and all(k in row for k in ['inode','device','links','mtimeSeconds','mtimeNanoseconds','atimeSeconds','atimeNanoseconds','ctimeSeconds','ctimeNanoseconds']),'Output metadata missing')
            if row['type']=='file':
                value=decode(row['dataBase64']);size_sum+=len(value)
                require(len(value)==row['bytes'] and hashlib.sha256(value).hexdigest()==row['sha256'],'Output file bytes/hash differ')
            elif row['type']=='symlink':decode(row['targetBase64'])
            else:require(row['type']=='directory','Special output type')
        require(len(paths)==len(set(paths))==effects['distEntries'] and size_sum==effects['distBytes'],'Full output tree count differs')
        if name!='background':require(paths.count(b'')==1,'Dist root omitted')
        if name=='boundary':
            check=json.loads(decode(effects['stdoutBase64']))
            require(check['count']==15 and check['failed']==0 and len(check['checks'])==15 and
                    {r['name'] for r in check['checks']}==helper.NAMES and all(r['pass'] is True for r in check['checks']),'Native negative controls differ')
        phases.append({'name':name,'seconds':result['seconds'],'commandComplete':response['complete'],
                       'expectedBackgroundRejection':name=='background','stdoutBytes':len(decode(effects['stdoutBase64'])),
                       'stderrBytes':len(decode(effects['stderrBase64'])),'distEntries':effects['distEntries'],
                       'distBytes':effects['distBytes'],'effectsSha256':result['effectsSha256'],'remainingProcesses':proof['remainingProcesses']})
    require(data['build-1']['effects']==data['build-2']['effects'],'Actual full build effects differ')
    require(data['build-1']['proof']['entropyProbeBase64']==data['build-2']['proof']['entropyProbeBase64'],'Repeated build kernel entropy differs')
    retained=read(EVIDENCE/'files.json')
    actual={p.relative_to(EVIDENCE).as_posix():{'sha256':sha(p),'bytes':p.stat().st_size}
            for p in sorted(EVIDENCE.rglob('*')) if p.is_file() and p.name!='files.json'}
    require(retained==actual,'Evidence file manifest differs')
    report.update(componentsPassed=True,archive=transport,image=archive,policy=policy,phases=phases,
                  repeatedBuild={'rawEffectsDeepEqual':True,'stdoutStderrExact':True,'fullDistMetadataExact':True,
                                 'kernelEntropyEqual':True,'effectsSha256':next(r['effectsSha256'] for r in phases if r['name']=='build-1')},
                  trustedHelperSha256=helper_sha,
                  files={p.relative_to(ROOT).as_posix():{'bytes':p.stat().st_size,'sha256':sha(p)} for p in sorted(ROOT.rglob('*')) if p.is_file()})
    require(sha(helper_path)==helper_sha,'Trusted data reader changed during audit')
except Exception as exc:report['issues'].append(str(exc));report['componentsPassed']=False
OUTPUT.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k!='files'},indent=2))
raise SystemExit(0 if report.get('componentsPassed') else 1)

