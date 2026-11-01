import Link from "next/link";
import { RANGES, type Range } from "@/lib/api/types";
import { RANGE_LABEL } from "@/lib/params";

/** Last 7 days / 30 days / 90 days. Links, so every range has its own URL. */
export function RangeToggle({ base, current }: { base: string; current: Range }) {
  return (
    <nav className="seg" aria-label="Time range">
      {RANGES.map((r) => (
        <Link key={r} href={r === "7d" ? base : `${base}?range=${r}`} scroll={false} aria-current={r === current ? "true" : undefined} replace>
          {RANGE_LABEL[r]}
        </Link>
      ))}
    </nav>
  );
}
