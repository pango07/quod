// The launcher prepends this only for a recorded run using stat proofs.
// Restore the exact original environment before any user preload executes.
'use strict'
const marker='--require="'+__filename.replace(/\\/g,'\\\\').replace(/"/g,'\\"')+'"'
const options=process.env.NODE_OPTIONS
if(options===marker) delete process.env.NODE_OPTIONS
else if(options?.startsWith(marker+' ')) process.env.NODE_OPTIONS=options.slice(marker.length+1)
const origin=require('./stat-origin.cjs')
// Bind the ESM proof and its parser-intrinsic snapshots before user code.
// Later edits to require.cache cannot replace that captured origin object.
// Older Node versions without synchronous require(ESM) retain metadata.
try {require('./boolean-proof.mjs')} catch {origin.disable()}
