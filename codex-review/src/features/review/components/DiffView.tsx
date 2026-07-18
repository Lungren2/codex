import { PatchDiff, type SelectedLineRange } from "@pierre/diffs/react";
import { useEffect, useRef } from "react";
import type { DiffHunk, FeedbackScope } from "../../../contracts/review";

type Props = {
  hunks: DiffHunk[];
  mode: "unified" | "split";
  activeScope: FeedbackScope;
  onSelectHunk: (hunk: DiffHunk) => void;
  onActiveHunk?: (hunk: DiffHunk) => void;
  onSelectLines: (hunk: DiffHunk, side: "old" | "new", startLine: number, endLine: number) => void;
};

export function DiffView({ hunks, mode, activeScope, onSelectHunk, onActiveHunk, onSelectLines }: Props) {
  const nodes = useRef(new Map<string, HTMLElement>());

  useEffect(() => {
    if (!onActiveHunk) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort((left, right) => Math.abs(left.boundingClientRect.top - 120) - Math.abs(right.boundingClientRect.top - 120))[0];
        const hunk = hunks.find(({ id }) => id === visible?.target.id);
        if (hunk) onActiveHunk(hunk);
      },
      { root: document.querySelector(".review-content"), rootMargin: "-5% 0px -72% 0px", threshold: 0 },
    );
    nodes.current.forEach((node) => observer.observe(node));
    return () => observer.disconnect();
  }, [hunks, onActiveHunk]);

  if (hunks.length === 0) {
    return <div className="diff-empty">No textual hunks are attached. Open Complete diff to inspect all captured changes.</div>;
  }

  return (
    <div className="diff-stack">
      {hunks.map((hunk) => {
        const active = (activeScope.type === "hunk" || activeScope.type === "lines") && activeScope.hunkId === hunk.id;
        return (
          <article
            className={`diff-file pierre-diff ${active ? "diff-file--active" : ""}`}
            key={hunk.id}
            id={hunk.id}
            ref={(node) => { if (node) nodes.current.set(hunk.id, node); else nodes.current.delete(hunk.id); }}
            onFocusCapture={() => onSelectHunk(hunk)}
          >
            {hunk.containsSecret ? (
              <div className="secret-warning">Content withheld because it resembles a credential. The frozen local patch remains authoritative.</div>
            ) : (
              <PatchDiff
                patch={toPatch(hunk)}
                options={{
                  theme: "pierre-dark",
                  themeType: "dark",
                  diffStyle: mode,
                  diffIndicators: "bars",
                  hunkSeparators: "line-info",
                  overflow: "scroll",
                  enableLineSelection: true,
                  lineHoverHighlight: "line",
                  onLineSelectionEnd: (range) => selectRange(hunk, range, onSelectLines),
                }}
              />
            )}
          </article>
        );
      })}
    </div>
  );
}

function toPatch(hunk: DiffHunk) {
  const oldPath = `a/${hunk.path}`;
  const newPath = `b/${hunk.path}`;
  const body = hunk.lines.map((line) => {
    const prefix = line.kind === "addition" ? "+" : line.kind === "deletion" ? "-" : " ";
    return `${prefix}${line.content}`;
  }).join("\n");
  return `diff --git ${oldPath} ${newPath}\n--- ${oldPath}\n+++ ${newPath}\n${hunk.header}\n${body}\n`;
}

function selectRange(
  hunk: DiffHunk,
  range: SelectedLineRange | null,
  onSelectLines: Props["onSelectLines"],
) {
  if (!range) return;
  const side = range.side === "deletions" ? "old" : "new";
  onSelectLines(hunk, side, Math.min(range.start, range.end), Math.max(range.start, range.end));
}
