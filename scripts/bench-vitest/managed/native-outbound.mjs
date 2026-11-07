// Deliberately omit the JS prelude. Verify the runtime's outbound policy
// independently, using native fetch rather than the managed API guard.
export default {
  async fetch(request) {
    if (new URL(request.url).pathname === '/health') return Response.json({ready: true})
    try {
      const response = await fetch('https://quod-denied.invalid/native-boundary-check')
      return Response.json({nativeOutboundStatus: response.status, body: await response.text()})
    } catch (error) {
      return Response.json({nativeOutboundError: String(error)})
    }
  },
}
