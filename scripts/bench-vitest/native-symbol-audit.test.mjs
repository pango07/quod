import {test} from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import crypto from 'node:crypto'
import {spawnSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import {readNativeSymbolAudit} from './native-symbol-report.mjs'

const auditSource=fileURLToPath(new URL('./native-symbol-audit.c',import.meta.url))
function command(file,args,options={}) {
  const r=spawnSync(file,args,{encoding:'utf8',timeout:30000,...options})
  assert.equal(r.error,undefined);assert.equal(r.status,0,r.stderr)
  return r
}
test('loader audit observes dynamic addon lookups, preserves results, and records exact paths',()=>{
  assert.equal(process.platform,'linux','compile and run only in the GitHub Linux gate')
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'quod-native-audit-'))
  try {
    const audit=path.join(dir,'audit.so'),source=path.join(dir,'fixture.c')
    // Tabs and percent signs exercise the lossless log encoding too.
    const addon=path.join(dir,'native\t% fixture.node'),logs=path.join(dir,'logs')
    fs.mkdirSync(logs)
    fs.writeFileSync(source,`#define _GNU_SOURCE
#include <node_api.h>
#include <dlfcn.h>
#include <unistd.h>
__attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env,napi_value exports) {
  napi_status (*create)(napi_env,uint32_t,napi_value*)=dlsym(RTLD_DEFAULT,"napi_create_uint32");
  napi_status (*set)(napi_env,napi_value,const char*,napi_value)=dlsym(RTLD_DEFAULT,"napi_set_named_property");
  pid_t (*pid)(void)=dlsym(RTLD_DEFAULT,"getpid");
  if(!create||!set||!pid||pid()<=0)return NULL;
  napi_value value; if(create(env,7013,&value)!=napi_ok)return NULL;
  if(set(env,exports,"answer",value)!=napi_ok)return NULL;
  return exports;
}
`)
    command('cc',['-std=c11','-Wall','-Wextra','-Werror','-shared','-fPIC','-O2',auditSource,'-o',audit])
    command('cc',['-std=c11','-Wall','-Wextra','-Werror','-shared','-fPIC','-O2',
      '-I'+path.resolve(path.dirname(process.execPath),'../include/node'),source,'-ldl','-o',addon])
    const args=['-e',`console.log(JSON.stringify(require(${JSON.stringify(addon)})))`]
    const plain=command(process.execPath,args)
    const traced=command(process.execPath,args,{env:{...process.env,LD_AUDIT:audit,QUOD_NATIVE_AUDIT_DIR:logs}})
    assert.equal(plain.stdout,'{"answer":7013}\n');assert.equal(traced.stdout,plain.stdout)
    const report=readNativeSymbolAudit(logs)
    assert.equal(report.replayEligible,false);assert.equal(report.addons.length,1)
    const observed=report.addons[0]
    assert.equal(observed.path,addon)
    assert.equal(observed.postRunSha256,crypto.createHash('sha256').update(fs.readFileSync(addon)).digest('hex'))
    for(const name of ['napi_create_uint32','napi_set_named_property','getpid'])
      assert.ok(observed.dynamicSymbols.includes(name),'missing dynamic resolution '+name)
    assert.ok(observed.linkedSymbols.includes('dlsym'))
    assert.ok(observed.bindings.find(b=>b.symbol==='getpid'&&b.dynamic&&/libc\.so/.test(b.provider)))
    assert.ok(observed.bindings.find(b=>b.symbol==='napi_create_uint32'&&b.dynamic&&b.provider===''))
    // The collector must not quietly treat missing/corrupt diagnostics as a
    // clean native boundary, or label a deleted loaded file as verified.
    fs.unlinkSync(addon)
    const absent=readNativeSymbolAudit(logs).addons[0]
    assert.equal(absent.postRunSha256,null);assert.ok(absent.postRunHashError)
    fs.writeFileSync(path.join(logs,'loader-0.tsv'),'bind\t0')
    assert.throws(()=>readNativeSymbolAudit(logs),/truncated/)
  }finally{fs.rmSync(dir,{recursive:true,force:true})}
})
