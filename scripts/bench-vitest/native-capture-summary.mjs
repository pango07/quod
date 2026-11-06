// Version 1 of the native helper's compact, sanitized response. This is a
// wire encoding only; every validity, cleanup and target-inspection field
// is restored before the existing observer checks consume the summary.
const array=Array.isArray,safe=Number.isSafeInteger
export function decodeNativeCaptureSummary(value) {
  if(!array(value)||value.length!==4||value[0]!==1||!safe(value[1])||value[1]<0||value[1]>=2**20)
    throw Error('invalid packed native capture summary')
  const [,bits,api,count]=value
  const owned=!!(bits&1),metadataValid=!!(bits&8),kind=(bits>>>7)&3,mask=(bits>>>9)&7,
    failure=(bits>>>12)&7,rejection=(bits>>>15)&7,excludedOwner=(bits>>>18)&3
  if(kind>2||failure>6||rejection>5||excludedOwner>2)
    throw Error('invalid packed native capture classification')
  if(metadataValid&&!owned)throw Error('invalid packed native capture ownership')
  if(owned&&metadataValid) {
    if(typeof api!=='string'||!safe(count)||count<0)throw Error('invalid packed native capture metadata')
  }else if(api!==null||count!==0||(bits&0xff0)!==0||excludedOwner!==0)
    throw Error('invalid packed native capture empty metadata')
  return {owned,available:!!(bits&2),cleared:!!(bits&4),metadataValid,excludedOwner,failure,rejection,
    ...(owned&&metadataValid?{api,argumentCount:count,receiverBased:!!(bits&16),describeOwnAccessors:!!(bits&32),
      kind,inspected:!!(bits&64),mask}:{})}
}
