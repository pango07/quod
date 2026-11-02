#!/usr/bin/env python3
"""Per recorded manifest of the demo's trusted scope: entry count and the
workspace prefixes it read (first two path components)."""
import json, sqlite3, sys, collections
db = sqlite3.connect("/var/lib/quod-gha/scopes/pango07_quod-demo-moto/trusted/ledger.sqlite")
for mid, key, created, blob in db.execute("select id, step_key, created_at, json from manifests order by created_at"):
    try:
        m = json.loads(blob)
    except Exception as e:
        print(mid, "undecodable", e); continue
    ents = m.get("entries") or m.get("inputs") or []
    c = collections.Counter()
    for e in ents:
        p = e.get("p") if isinstance(e, dict) else str(e)
        c["/".join(p.split("/")[:2])] += 1
    print(created[:19], mid[:12], len(ents), dict(c.most_common(int(sys.argv[1]) if len(sys.argv) > 1 else 8)))
