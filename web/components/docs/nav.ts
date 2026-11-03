/** The docs sidebar: groups of pages, in reading order. Prev/next links follow this order too. */
export type DocLink = { title: string; href: string; soon?: boolean };
export type DocGroup = { title: string; links: DocLink[] };

export const DOCS_NAV: DocGroup[] = [
  {
    title: "Getting started",
    links: [
      { title: "Introduction", href: "/docs" },
      { title: "Quickstart", href: "/docs/quickstart" },
    ],
  },
  {
    title: "Concepts",
    links: [
      { title: "How it works", href: "/docs/how-it-works" },
      { title: "Trust and audits", href: "/docs/trust" },
      { title: "Supported setups", href: "/docs/supported-setups" },
    ],
  },
  {
    title: "Guides",
    links: [
      { title: "Make it even faster", href: "/docs/faster" },
      { title: "Quod Cloud", href: "/docs/cloud" },
      { title: "Troubleshooting and FAQ", href: "/docs/troubleshooting" },
    ],
  },
  {
    title: "Reference",
    links: [
      { title: "GitHub Action", href: "/docs/reference/action" },
      { title: "Environment variables", href: "/docs/reference/environment-variables" },
      { title: "quod vitest CLI", href: "/docs/reference/cli" },
    ],
  },
];

export const DOCS_FLAT: DocLink[] = DOCS_NAV.flatMap((g) => g.links);
