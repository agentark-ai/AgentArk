import type { ReactNode } from "react";

const CONSOLE_LINK_PATTERN =
  /\b((?:https?:\/\/|www\.)[^\s<>"'`]+)(?=$|\s|[<>"'`])/gi;

function splitTrailingPunctuation(value: string): [string, string] {
  let core = value;
  let trailing = "";
  while (core && /[.,;:!?)]$/.test(core)) {
    trailing = `${core.slice(-1)}${trailing}`;
    core = core.slice(0, -1);
  }
  return [core, trailing];
}

function hrefForConsoleLink(value: string): string {
  return /^https?:\/\//i.test(value) ? value : `https://${value}`;
}

export function renderLinkifiedText(
  text: string,
  linkClassName = "cview-inline-link",
): ReactNode[] {
  const value = text || "";
  if (!value) return [];
  const nodes: ReactNode[] = [];
  let lastIndex = 0;

  for (const match of value.matchAll(CONSOLE_LINK_PATTERN)) {
    const matched = match[0] || "";
    const index = match.index ?? 0;
    if (!matched) continue;
    if (index > lastIndex) {
      nodes.push(value.slice(lastIndex, index));
    }
    const [core, trailing] = splitTrailingPunctuation(matched);
    if (core) {
      nodes.push(
        <a
          key={`link-${index}-${core}`}
          className={linkClassName}
          href={hrefForConsoleLink(core)}
          target="_blank"
          rel="noopener noreferrer"
        >
          {core}
        </a>,
      );
    }
    if (trailing) nodes.push(trailing);
    lastIndex = index + matched.length;
  }

  if (lastIndex < value.length) {
    nodes.push(value.slice(lastIndex));
  }
  return nodes.length > 0 ? nodes : [value];
}

export function LinkifiedText({
  text,
  linkClassName,
}: {
  text: string;
  linkClassName?: string;
}) {
  return <>{renderLinkifiedText(text, linkClassName)}</>;
}
