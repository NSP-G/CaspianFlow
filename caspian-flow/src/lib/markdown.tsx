import React from "react";

/**
 * Minimal, dependency-free Markdown → React renderer for the built-in help docs.
 *
 * Supports the small subset the help pages actually use:
 *   # / ## / ### headings, paragraphs, unordered lists (- or *), fenced code
 *   blocks (```), and inline `code`, **bold**, and [links](url).
 *
 * Kept intentionally tiny to honour the project's "minimal dependencies"
 * discipline — no markdown parser crate/library is pulled in.
 */

type Token =
  | { type: "h1" | "h2" | "h3"; text: string }
  | { type: "p"; text: string }
  | { type: "ul"; items: string[] }
  | { type: "code"; lang: string; code: string };

function parseMarkdown(md: string): Token[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const tokens: Token[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith("```")) {
      const lang = line.slice(3).trim();
      const buf: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        buf.push(lines[i]);
        i++;
      }
      i++; // skip closing fence
      tokens.push({ type: "code", lang, code: buf.join("\n") });
      continue;
    }
    if (line.startsWith("### ")) {
      tokens.push({ type: "h3", text: line.slice(4) });
      i++;
      continue;
    }
    if (line.startsWith("## ")) {
      tokens.push({ type: "h2", text: line.slice(3) });
      i++;
      continue;
    }
    if (line.startsWith("# ")) {
      tokens.push({ type: "h1", text: line.slice(2) });
      i++;
      continue;
    }
    if (line.startsWith("- ") || line.startsWith("* ")) {
      const items: string[] = [];
      while (i < lines.length && (lines[i].startsWith("- ") || lines[i].startsWith("* "))) {
        items.push(lines[i].slice(2));
        i++;
      }
      tokens.push({ type: "ul", items });
      continue;
    }
    if (line.trim() === "") {
      i++;
      continue;
    }
    // Paragraph: gather consecutive non-blank, non-structural lines.
    const buf: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("# ") &&
      !lines[i].startsWith("## ") &&
      !lines[i].startsWith("### ") &&
      !lines[i].startsWith("- ") &&
      !lines[i].startsWith("* ") &&
      !lines[i].startsWith("```")
    ) {
      buf.push(lines[i]);
      i++;
    }
    tokens.push({ type: "p", text: buf.join(" ") });
  }
  return tokens;
}

const INLINE_RE = /(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\([^)]+\))/g;

function renderInline(text: string, keyPrefix: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  let last = 0;
  let idx = 0;
  let m: RegExpExecArray | null;
  INLINE_RE.lastIndex = 0;
  while ((m = INLINE_RE.exec(text)) !== null) {
    if (m.index > last) nodes.push(text.slice(last, m.index));
    const tok = m[0];
    const key = `${keyPrefix}-${idx++}`;
    if (tok.startsWith("`")) {
      nodes.push(
        <code key={key} className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
          {tok.slice(1, -1)}
        </code>,
      );
    } else if (tok.startsWith("**")) {
      nodes.push(<strong key={key}>{tok.slice(2, -2)}</strong>);
    } else {
      const mm = /\[([^\]]+)\]\(([^)]+)\)/.exec(tok);
      if (mm) {
        nodes.push(
          <a
            key={key}
            href={mm[2]}
            className="text-accent underline underline-offset-2"
            target="_blank"
            rel="noreferrer"
          >
            {mm[1]}
          </a>,
        );
      }
    }
    last = m.index + tok.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

export function Markdown({ content, className }: { content: string; className?: string }) {
  const tokens = parseMarkdown(content);
  return (
    <div className={className ?? "space-y-3 text-sm leading-relaxed"}>
      {tokens.map((tok, ti) => {
        const key = `t-${ti}`;
        switch (tok.type) {
          case "h1":
            return (
              <h1 key={key} className="text-2xl font-semibold tracking-tight">
                {renderInline(tok.text, key)}
              </h1>
            );
          case "h2":
            return (
              <h2 key={key} className="mt-2 border-b border-border pb-1 text-lg font-semibold">
                {renderInline(tok.text, key)}
              </h2>
            );
          case "h3":
            return (
              <h3 key={key} className="text-base font-semibold text-foreground">
                {renderInline(tok.text, key)}
              </h3>
            );
          case "ul":
            return (
              <ul key={key} className="list-disc space-y-1 pl-6">
                {tok.items.map((item, ii) => (
                  <li key={`${key}-${ii}`}>{renderInline(item, `${key}-${ii}`)}</li>
                ))}
              </ul>
            );
          case "code":
            return (
              <pre
                key={key}
                className="overflow-x-auto rounded-lg border border-border bg-muted p-3 text-xs leading-relaxed"
              >
                <code className="font-mono">{tok.code}</code>
              </pre>
            );
          case "p":
          default:
            return (
              <p key={key} className="text-muted-foreground">
                {renderInline(tok.text, key)}
              </p>
            );
        }
      })}
    </div>
  );
}
