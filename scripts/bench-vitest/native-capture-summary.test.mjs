import test from 'node:test'
import assert from 'node:assert/strict'
import {decodeNativeCaptureSummary} from './native-capture-summary.mjs'

test('packed decoding accepts null-prototype arrays and never invokes an iterator',()=>{
  const plain=[1,15,'api',1],native=Object.setPrototypeOf([1,15,'api',1],null)
  let calls=0
  Object.defineProperty(plain,Symbol.iterator,{get(){calls++;throw Error('application iterator')}})
  assert.deepEqual(decodeNativeCaptureSummary(native),decodeNativeCaptureSummary(plain))
  assert.equal(calls,0)
})

test('packed summary decodes every boolean and classification without conflating bits',()=>{
  for(const receiver of [false,true])for(const describe of [false,true])for(const inspected of [false,true])
    for(let kind=0;kind<3;kind++)for(let mask=0;mask<8;mask++) {
      const bits=15+(receiver?16:0)+(describe?32:0)+(inspected?64:0)+(kind<<7)+(mask<<9)
      assert.deepEqual(decodeNativeCaptureSummary([1,bits,'api',Number.MAX_SAFE_INTEGER]),
        {owned:true,available:true,cleared:true,metadataValid:true,excludedOwner:0,failure:0,rejection:0,
          api:'api',argumentCount:Number.MAX_SAFE_INTEGER,receiverBased:receiver,describeOwnAccessors:describe,kind,inspected,mask})
    }
  for(let failure=0;failure<=6;failure++)for(let rejection=0;rejection<=5;rejection++)for(let excludedOwner=0;excludedOwner<=2;excludedOwner++) {
    const summary=decodeNativeCaptureSummary([1,9+(failure<<12)+(rejection<<15)+(excludedOwner<<18),'api',0])
    assert.equal(summary.failure,failure);assert.equal(summary.rejection,rejection);assert.equal(summary.excludedOwner,excludedOwner)
    assert.equal(summary.available,false);assert.equal(summary.cleared,false)
  }
  assert.deepEqual(decodeNativeCaptureSummary([1,2+(4<<15),null,0]),
    {owned:false,available:true,cleared:false,metadataValid:false,excludedOwner:0,failure:0,rejection:4})
})

test('packed summary rejects missing fields, version changes, invalid classifications and noninteger arity',()=>{
  for(const bad of [null,{},[],[1,15,'api'],[2,15,'api',1],[1,-1,'api',1],[1,2**20,'api',1],
    [1,NaN,'api',1],[1,15.5,'api',1],[1,15+(3<<7),'api',1],[1,15+(7<<12),'api',1],
    [1,15+(6<<15),'api',1],[1,15+(3<<18),'api',1],[1,15,null,1],
    [1,15,'api',NaN],[1,15,'api',-1],[1,15,'api',0.5],[1,15,'api',Number.MAX_SAFE_INTEGER+1],
    [1,2,'leaked label',0],[1,2,null,1],[1,2+16,null,0],[1,2+(1<<18),null,0],[1,8,null,0],
    [1,15,'api',1,'extra']])assert.throws(()=>decodeNativeCaptureSummary(bad))
})
