import type { MDXComponents } from "mdx/types";
import Link from "next/link";
import type { ComponentProps } from "react";
import { Badge, Callout, Card, Cards, FlowDiagram, Note, Soon, Stat, Stats, Steps, Tip, Warning } from "@/components/docs/blocks";
import { Pre } from "@/components/docs/pre";
import { Tabs } from "@/components/docs/tabs";

/** A heading with a hover anchor, as linked from "On this page". */
function anchored(Tag: "h2" | "h3") {
  return function Heading({ id, children, ...rest }: ComponentProps<"h2">) {
    return (
      <Tag id={id} {...rest}>
        {children}
        {id ? (
          <a href={`#${id}`} className="d-anchor" aria-label="Link to this section">
            #
          </a>
        ) : null}
      </Tag>
    );
  };
}

function A({ href = "", ...rest }: ComponentProps<"a">) {
  if (href.startsWith("/")) return <Link href={href} {...rest} />;
  if (href.startsWith("#")) return <a href={href} {...rest} />;
  return <a href={href} target="_blank" rel="noreferrer" {...rest} />;
}

/** A wide table scrolls on its own instead of widening the page. */
function Table(props: ComponentProps<"table">) {
  return (
    <div className="d-table">
      <table {...props} />
    </div>
  );
}

const components: MDXComponents = {
  h2: anchored("h2"),
  h3: anchored("h3"),
  a: A,
  pre: Pre,
  table: Table,
  Badge,
  Callout,
  Card,
  Cards,
  FlowDiagram,
  Note,
  Soon,
  Stat,
  Stats,
  Steps,
  Tabs,
  Tip,
  Warning,
};

export function useMDXComponents(): MDXComponents {
  return components;
}
