// Team-level pages. These names can never be repository slugs (the backend reserves them).
export const TEAM_PAGES = ["repositories", "runs", "trust", "runners", "usage", "settings", "connect", "events"];

export const NAV = [
  { href: "", label: "Overview", icon: "overview" },
  { href: "/repositories", label: "Repositories", icon: "repos" },
  { href: "/runs", label: "Runs", icon: "runs" },
  { href: "/trust", label: "Trust", icon: "trust" },
  { href: "/runners", label: "Runners", icon: "runners" },
  { href: "/usage", label: "Usage", icon: "usage" },
  { href: "/settings", label: "Settings", icon: "settings" },
] as const;
