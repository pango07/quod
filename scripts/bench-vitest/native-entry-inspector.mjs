// Diagnostic only. The helper lives behind a private Inspector handle;
// nothing is installed on application globals or exported to application JS.
// Captured intrinsics do not certify initial runtime integrity or exclude
// exotic native interceptors / foreign Inspector sessions.
const factory=`(nonce=>{
  'use strict';
  const descriptor=Object.getOwnPropertyDescriptor,has=Object.hasOwn,keys=Reflect.ownKeys,
    define=Reflect.defineProperty,isProxy=process.getBuiltinModule('util').types.isProxy,
    safeInteger=Number.isSafeInteger,setPrototype=Object.setPrototypeOf;
  const configurations={__proto__:null};
  function object(value){return value!==null&&(typeof value==='object'||typeof value==='function')}
  function data(value,key){
    if(!object(value)||isProxy(value))throw 0;
    const d=descriptor(value,key);
    if(!d||!has(d,'value'))throw 0;
    return d.value;
  }
  function capture(exception){
    const result={__proto__:null,owned:false,available:false,cleared:false};
    let canClear=false;
    try{
      if(!object(exception)||isProxy(exception))return result;
      const marker=descriptor(exception,'quodNativeEntry');
      if(!marker||!has(marker,'value')||marker.value!==nonce){result.available=true;return result}
      result.owned=true;
      const names=['quodNativeEntry','api','args','receiver'];
      for(let i=0;i<names.length;i++){
        const d=descriptor(exception,names[i]);
        if(!d||!has(d,'value')||d.writable!==true)throw 0;
      }
      canClear=true;
      const api=data(exception,'api');
      if(typeof api!=='string')throw 0;
      const configuration=data(configurations,api),args=data(exception,'args'),argumentCount=data(args,'length');
      if(!safeInteger(argumentCount)||argumentCount<0)throw 0;
      const argument=configuration.receiver?data(exception,'receiver'):argumentCount?data(args,'0'):undefined;
      const kind=isProxy(argument)?'proxy':object(argument)?'object':'primitive';
      result.api=api;result.argumentCount=argumentCount;result.receiverBased=configuration.receiver;result.kind=kind;
      if(configuration.describeOwnAccessors&&kind==='object'){
        let ownAccessorCount=0;
        const watchedAccessorKeys=[],names=keys(argument);
        setPrototype(watchedAccessorKeys,null);
        for(let i=0;i<names.length;i++){
          const key=names[i],d=descriptor(argument,key);
          if(d&&((has(d,'get')&&typeof d.get==='function')||(has(d,'set')&&typeof d.set==='function'))){
            ownAccessorCount++;
            if(key==='seed'||key==='tmp'||key==='invalidate')define(watchedAccessorKeys,watchedAccessorKeys.length,
              {__proto__:null,value:key,enumerable:true,configurable:true,writable:true});
          }
        }
        result.target={__proto__:null,ownAccessorCount,watchedAccessorKeys};
      }
      result.available=true;
    }catch{result.available=false}
    finally{
      if(canClear){
        // These are validated own writable data fields on our fresh wrapper.
        // Clear it before returning so console history retains no arguments.
        exception.args=null;exception.receiver=null;exception.quodNativeEntry=null;exception.api=null;
        result.cleared=true;
      }
    }
    return result;
  }
  return {__proto__:null,capture,register(api,receiver,describeOwnAccessors){
    if(typeof api!=='string'||has(configurations,api)||typeof receiver!=='boolean'||typeof describeOwnAccessors!=='boolean')throw 0;
    configurations[api]={__proto__:null,receiver,describeOwnAccessors};return true;
  }};
})`

export function privateNativeEntryInspector(post,nonce) {
  const group='quod-native-inspector-'+nonce
  let handle=null,closed=false
  function call(functionDeclaration,args,returnByValue=true) {
    if(closed||!handle)throw Error('private native inspector unavailable')
    const r=post('Runtime.callFunctionOn',{objectId:handle,functionDeclaration,arguments:args,
      silent:true,returnByValue,generatePreview:false})
    if(r.exceptionDetails)throw Error('private native inspection failed')
    return r.result?.value
  }
  return {
    register(api,{receiver,describeOwnAccessors}) {
      if(closed)throw Error('private native inspector closed')
      if(!handle) {
        const r=post('Runtime.evaluate',{expression:factory+'('+JSON.stringify(nonce)+')',objectGroup:group,silent:true})
        if(r.exceptionDetails||!r.result?.objectId)throw Error('private native inspector setup failed')
        handle=r.result.objectId
      }
      if(call('function(api,receiver,describe){return this.register(api,receiver,describe)}',
        [{value:api},{value:receiver},{value:describeOwnAccessors}])!==true)throw Error('private native registration failed')
    },
    capture(objectId) {
      const result=call('function(exception){return this.capture(exception)}',[{objectId}])
      if(!result||typeof result.owned!=='boolean'||typeof result.available!=='boolean'||typeof result.cleared!=='boolean')throw Error('invalid private native summary')
      return result
    },
    close(){if(closed)return;closed=true;post('Runtime.releaseObjectGroup',{objectGroup:group})},
  }
}
