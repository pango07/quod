// Diagnostic construction rule, not replay authority. Captures may exclude
// API-wrapper/interceptor targets only if every watched scalar owner passes
// this native guard before descriptor inspection or accessor installation.
import path from 'node:path'
import {createRequire} from 'node:module'
const require=createRequire(import.meta.url)
export function nativeOwnerGuard() {
  const addon=process.env.QUOD_NATIVE_CAPTURE_ADDON
  if(process.versions.node!=='24.18.0'||!addon||!path.isAbsolute(addon))throw Error('native owner guard requires the pinned addon')
  const supported=require(addon).isInspectableOwner
  if(typeof supported!=='function')throw Error('native owner guard unavailable')
  return object=>supported(object)===true
}
