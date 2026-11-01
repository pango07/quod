import Link from "next/link";

// A repo, run or step that does not exist, shown inside the dashboard frame.
export default function NotFound() {
  return (
    <div className="page">
      <p className="mono faint">404</p>
      <h1 style={{ fontSize: 24, fontWeight: 600, letterSpacing: "-0.02em" }}>Nothing here</h1>
      <p className="muted" style={{ fontSize: 15, lineHeight: 1.6, maxWidth: "60ch" }}>
        This repository, run or step is not in your team. It may have been renamed, or the link is from another team.
      </p>
      <div>
        <Link className="btn solid" href="/">
          Back to the overview
        </Link>
      </div>
    </div>
  );
}
