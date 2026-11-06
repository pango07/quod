// Diagnostic only. A private Inspector function inspects exactly three own
// descriptors and returns a primitive bit mask. It never reads their values,
// invokes getters, enumerates the target, or retains application references.
// Captured intrinsics do not prove initial runtime integrity or exclude
// exotic native interceptors and foreign Inspector sessions.
const integer=Number.isInteger
const factory=`(()=>{
  'use strict';
  const descriptor=Object.getOwnPropertyDescriptor,has=Object.hasOwn,
    isProxy=process.getBuiltinModule('util').types.isProxy;
  return function(target){
    if(target===null||(typeof target!=='object'&&typeof target!=='function')||isProxy(target))throw 0;
    function accessor(key){
      const d=descriptor(target,key);
      return !!d&&((has(d,'get')&&typeof d.get==='function')||(has(d,'set')&&typeof d.set==='function'));
    }
    return (accessor('seed')?1:0)|(accessor('tmp')?2:0)|(accessor('invalidate')?4:0);
  };
})()`

export function watchedAccessorInspector(post,nonce) {
  const group='quod-watched-accessors-'+nonce
  let handle=null,closed=false
  return {
    initialize() {
      if(closed)throw Error('watched accessor inspector closed')
      if(handle)return
      const r=post('Runtime.evaluate',{expression:factory,objectGroup:group,silent:true,generatePreview:false})
      if(r.exceptionDetails||!r.result?.objectId)throw Error('watched accessor setup failed')
      handle=r.result.objectId
    },
    inspect(objectId) {
      if(closed||!handle)throw Error('watched accessor inspector unavailable')
      const r=post('Runtime.callFunctionOn',{objectId:handle,
        functionDeclaration:'function(target){return this(target)}',arguments:[{objectId}],
        returnByValue:true,silent:true,generatePreview:false})
      if(r.exceptionDetails)throw Error('watched accessor inspection failed')
      const mask=r.result?.value
      if(!integer(mask)||mask<0||mask>7)throw Error('invalid watched accessor result')
      const watchedAccessorKeys=['seed','tmp','invalidate'].filter((_,i)=>mask&(1<<i))
      return {inspectionScope:'watched-properties',watchedAccessorCount:watchedAccessorKeys.length,watchedAccessorKeys}
    },
    close(){if(closed)return;closed=true;if(handle)post('Runtime.releaseObjectGroup',{objectGroup:group})},
  }
}
