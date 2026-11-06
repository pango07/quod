import {test} from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import crypto from 'node:crypto'
import {spawnSync} from 'node:child_process'

const probe=new URL('./native-load-probe.mjs',import.meta.url).href
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
function command(file,args,options={}) {
  const r=spawnSync(file,args,{encoding:'utf8',timeout:30000,...options})
  assert.equal(r.error,undefined);assert.equal(r.status,0,r.stderr||r.stdout)
  return r
}
test('native load identity rejects changed artifacts and preserves the actual loader result',()=>{
  assert.equal(process.platform,'linux','run native compilation in the GitHub gate')
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'quod-native-load-'))
  try {
    const source=path.join(dir,'fixture.c'),first=path.join(dir,'first.node'),second=path.join(dir,'second.node')
    fs.writeFileSync(source,`#include <node_api.h>
#include <stdio.h>
#include <stdlib.h>
#ifndef ANSWER
#define ANSWER 1
#endif
__attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env,napi_value exports) {
  const char *from=getenv("QUOD_FIXTURE_REPLACE_FROM"),*to=getenv("QUOD_FIXTURE_REPLACE_TO");
  if(from&&to&&rename(from,to)!=0)return NULL;
  napi_value value;if(napi_create_uint32(env,ANSWER,&value)!=napi_ok)return NULL;
  if(napi_set_named_property(env,exports,"answer",value)!=napi_ok)return NULL;
  return exports;
}
`)
    for(const [answer,file] of [[1,first],[2,second]])command('cc',['-std=c11','-Wall','-Wextra','-Werror','-shared','-fPIC','-O2',
      '-DANSWER='+answer,'-I'+path.resolve(path.dirname(process.execPath),'../include/node'),source,'-o',file])
    for(const kind of ['known','unknown','changed','replaced','restore','during-load','failure']) {
      const target=path.join(dir,kind+'.node'),replacement=path.join(dir,kind+'-replacement.node')
      fs.copyFileSync(first,target);fs.copyFileSync(second,replacement)
      const expected=kind==='during-load'?hash(second):kind==='unknown'?'0'.repeat(64):hash(first)
      const code=`import fs from 'node:fs';import crypto from 'node:crypto';import {createRequire} from 'node:module';
import {observeNativeLoads} from ${JSON.stringify(probe)};
const require=createRequire(import.meta.url),target=${JSON.stringify(target)},replacement=${JSON.stringify(replacement)},kind=${JSON.stringify(kind)};
const original=process.dlopen,probe=observeNativeLoads({digests:[${JSON.stringify(expected)}]});
let answer=null,caught=false;
try{if(kind==='failure')process.dlopen({exports:{}},target+'.missing');else answer=require(target).answer}catch{caught=true}
const initial=probe.report();
if(kind==='changed')fs.appendFileSync(target,'padding');
if(kind==='replaced')fs.renameSync(replacement,target);
if(kind==='restore'){fs.chmodSync(target,0o600);probe.snapshot();fs.chmodSync(target,0o755)}
const final=probe.report(),postHash=crypto.createHash('sha256').update(fs.readFileSync(target)).digest('hex');
probe.close();console.log(JSON.stringify({answer,caught,initial,final,postHash,restored:process.dlopen===original}));`
      const env={...process.env}
      if(kind==='during-load'){env.QUOD_FIXTURE_REPLACE_FROM=replacement;env.QUOD_FIXTURE_REPLACE_TO=target}
      const result=JSON.parse(command(process.execPath,['--input-type=module','-e',code],{env}).stdout)
      assert.equal(result.restored,true);assert.equal(result.final.replayEligible,false)
      if(kind==='failure'){assert.equal(result.caught,true);assert.equal(result.final.matchedArtifacts,0);continue}
      assert.equal(result.caught,false);assert.equal(result.answer,1,'the actual loaded addon must decide the result')
      assert.equal(result.initial.matchedArtifacts,['known','changed','replaced','restore'].includes(kind)?1:0,JSON.stringify(result))
      assert.equal(result.final.matchedArtifacts,kind==='known'?1:0,JSON.stringify(result))
      if(kind==='known') {
        const record=result.final.records[0]
        assert.equal(record.noAtimeRead,true);assert.equal(record.sha256,expected)
        assert.equal(record.mappedIdentity,true);assert.ok(record.mappings.length>0)
      }
      if(kind==='during-load') {
        assert.equal(result.postHash,expected,'a post-run hash alone would accept the replacement')
        assert.notEqual(result.initial.records[0].sha256,expected,'the pre-load hash identifies the actual different addon')
      }
    }
  }finally{fs.rmSync(dir,{recursive:true,force:true})}
})
