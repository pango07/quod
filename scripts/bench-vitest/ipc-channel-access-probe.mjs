// Owned Inspector callback source. `this` is the captured native data-field
// factory; the caller supplies the channel and exact Symbols from pinned Node
// closures. Host admission here depends on that caller check. This does not
// attest startup integrity, earlier aliases or all ways to acquire raw bytes.
export const ipcChannelObserverFactorySource=`function(channel,buffer,size) {
  'use strict';
  const factory=this,errors=new Set(),fields=[[buffer,'buffers'],[size,'size'],['buffering','buffering']];
  let watchers=[],armed=false,closed=false,arms=0,suspends=0,reads=0,installedFields=0,releasedFields=0;
  const byField={buffers:0,size:0,buffering:0};
  function validate() {
    for(const item of watchers) {
      const r=item.observer.report();
      if(!r.available)errors.add('channel field callback lost');
      if(r.calls!==item.reads)errors.add('channel field read accounting differs');
    }
  }
  function release() {
    for(const item of watchers) {
      item.observer.close();
      const r=item.observer.report();
      if(!r.restored)errors.add('channel field callback could not be restored');
      releasedFields++;
    }
    watchers=[];armed=false;
  }
  function arm() {
    if(closed||armed){errors.add('channel field guard arm out of order');return false}
    try {
      for(const [key,label] of fields) {
        const item={observer:null,reads:0};
        item.observer=factory(channel,key,()=>{
          item.reads++;reads++;byField[label]++;
          errors.add('channel field read outside native callback');
        },true);
        watchers.push(item);installedFields++;
      }
      armed=true;arms++;validate();
    }catch{errors.add('channel field guard installation failed');release()}
    return armed&&errors.size===0;
  }
  function suspend() {
    if(closed||!armed){errors.add('channel field guard suspend out of order');return false}
    validate();release();suspends++;
    return errors.size===0;
  }
  function report() {
    if(!closed&&armed)validate();
    return {available:!closed&&armed&&errors.size===0,armed,closed,arms,suspends,reads,
      byField:{...byField},installedFields,releasedFields,errors:[...errors],
      replayEligible:false,completeOwnership:false,earlierAliasesExcluded:false};
  }
  function close() {
    if(closed)return;
    if(armed){validate();release()}
    closed=true;
  }
  if(typeof buffer!=='symbol'||typeof size!=='symbol'||buffer===size)throw Error('channel keys unavailable');
  arm();
  return {arm,suspend,report,close};
}`
