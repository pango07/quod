import { Sk } from "@/components/ui";

// Loading states that keep the shape of each screen, so nothing jumps when data lands.

function StatRow({ n = 4, small }: { n?: number; small?: boolean }) {
  return (
    <div className={`stats ${small ? "sm" : ""}`}>
      {Array.from({ length: n }, (_, i) => (
        <div key={i} className="card stat">
          <Sk w={110} h={13} />
          <Sk w={90} h={small ? 26 : 32} />
          {small ? null : <Sk w={160} h={12} />}
        </div>
      ))}
    </div>
  );
}

function CardRows({ rows = 4, h = 18 }: { rows?: number; h?: number }) {
  return (
    <div className="card" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <Sk w={120} h={16} />
      </div>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="trow" style={{ gridTemplateColumns: "2fr 1fr 1fr" }}>
          <Sk w="70%" h={h} />
          <Sk w="60%" h={12} />
          <Sk w="40%" h={12} />
        </div>
      ))}
    </div>
  );
}

function Chart({ h = 180 }: { h?: number }) {
  return (
    <div className="card" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
      <Sk w={160} h={16} />
      <Sk h={h} r={8} />
    </div>
  );
}

const Busy = ({ children, tight }: { children: React.ReactNode; tight?: boolean }) => (
  <div className={`page ${tight ? "tight" : ""}`} aria-busy="true" aria-live="polite">
    <span className="sr-only">Loading</span>
    {children}
  </div>
);

export function OverviewSkeleton() {
  return (
    <Busy>
      <div className="row-between">
        <Sk w={150} h={32} />
        <Sk w={300} h={36} r={8} />
      </div>
      <StatRow />
      <Chart />
      <div className="split">
        <div style={{ flex: "999 1 560px" }}>
          <CardRows />
        </div>
        <div style={{ flex: "1 1 300px" }}>
          <CardRows rows={3} h={14} />
        </div>
      </div>
    </Busy>
  );
}

export function ListSkeleton({ title = 160, rows = 6 }: { title?: number; rows?: number }) {
  return (
    <Busy>
      <Sk w={title} h={32} />
      <CardRows rows={rows} />
    </Busy>
  );
}

export function RepoSkeleton() {
  return (
    <Busy>
      <div className="card" style={{ padding: 20, display: "flex", gap: 24, flexWrap: "wrap" }}>
        <div style={{ flex: "1 1 360px", display: "flex", flexDirection: "column", gap: 12 }}>
          {Array.from({ length: 6 }, (_, i) => (
            <Sk key={i} h={16} />
          ))}
        </div>
        <div style={{ flex: "1 1 320px", display: "grid", gridTemplateColumns: "1fr 1fr", gap: 18 }}>
          {Array.from({ length: 4 }, (_, i) => (
            <Sk key={i} h={34} />
          ))}
        </div>
      </div>
      <Chart h={160} />
      <CardRows rows={6} />
    </Busy>
  );
}

export function RunSkeleton() {
  return (
    <Busy tight>
      <Sk w="60%" h={30} />
      <Sk w={320} h={14} />
      <StatRow small />
      <div className="split">
        <div style={{ flex: "1 1 420px" }}>
          <CardRows rows={8} h={16} />
        </div>
        <div className="card" style={{ flex: "1 1 380px", padding: 20, display: "flex", flexDirection: "column", gap: 14 }}>
          <Sk w={180} h={16} />
          <Sk h={44} />
          <Sk h={120} r={8} />
        </div>
      </div>
    </Busy>
  );
}

export function StepSkeleton() {
  return (
    <Busy tight>
      <Sk w={60} h={13} />
      <Sk w={260} h={26} />
      <div className="card" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <Sk h={44} />
        <Sk h={8} />
        <Sk w={280} h={34} r={8} />
      </div>
      <div className="split">
        <div className="card" style={{ flex: "1 1 420px", padding: 20 }}>
          <Sk h={140} r={8} />
        </div>
        <div style={{ flex: "1 1 360px" }}>
          <CardRows rows={4} h={14} />
        </div>
      </div>
    </Busy>
  );
}
