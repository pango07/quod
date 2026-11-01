// Tiny self-contained HTML pages for the two setup steps that must not be
// React pages: the auto-submit to GitHub (it sets a cookie first) and the
// one-time display of the App's keys (no RSC payload, no cache, nothing kept).

export const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

const STYLE = `
:root{color-scheme:dark}
body{margin:0;background:#000;color:#ededed;font:15px/1.5 ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:560px;margin:0 auto;padding:40px 16px 64px;display:flex;flex-direction:column;gap:20px;box-sizing:border-box}
h1{font-size:24px;font-weight:600;letter-spacing:-.02em;margin:0}
p{margin:0;color:#a1a1a1}
.warn{border:1px solid #5c3d06;background:#1a1205;border-radius:10px;padding:12px 14px;color:#f5c46b}
.var{border:1px solid #262626;border-radius:12px;background:#0a0a0a;padding:14px;display:flex;flex-direction:column;gap:10px}
.row{display:flex;align-items:center;justify-content:space-between;gap:12px}
.name{font:600 13px ui-monospace,"SF Mono",Menlo,monospace;overflow-wrap:anywhere}
.tag{font-size:11px;color:#a1a1a1;border:1px solid #333;border-radius:999px;padding:1px 8px;white-space:nowrap}
.val{font:12px/1.5 ui-monospace,"SF Mono",Menlo,monospace;background:#050505;border:1px solid #1f1f1f;border-radius:8px;padding:10px;white-space:pre-wrap;word-break:break-all;max-height:140px;overflow:auto;margin:0;color:#d4d4d4}
button,.btn{display:inline-flex;align-items:center;justify-content:center;min-height:44px;padding:0 16px;border-radius:8px;border:1px solid #ededed;background:#ededed;color:#000;font:500 15px inherit;cursor:pointer;text-decoration:none}
.ghost{background:#0a0a0a;color:#ededed;border-color:#333}
ol{margin:0;padding-left:20px;color:#a1a1a1;display:flex;flex-direction:column;gap:6px}
ol strong{color:#ededed;font-weight:500}
a{color:#ededed}
`;

export function htmlPage(title: string, body: string, script = ""): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><meta name="referrer" content="no-referrer"><title>${esc(title)} · Quod Cloud</title><style>${STYLE}</style></head><body><main>${body}</main>${script ? `<script>${script}</script>` : ""}</body></html>`;
}

export const NO_STORE_HEADERS = {
  "content-type": "text/html; charset=utf-8",
  "cache-control": "no-store, max-age=0",
  pragma: "no-cache",
  "referrer-policy": "no-referrer",
  "x-robots-tag": "noindex",
} as const;
