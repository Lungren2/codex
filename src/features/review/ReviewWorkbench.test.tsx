// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@pierre/diffs/react", () => ({
  PatchDiff: ({ options }: { options: { onLineSelectionEnd(range: unknown): void } }) => (
    <div role="table" aria-label="Pierre split diff">
      <button role="row" onClick={() => options.onLineSelectionEnd({ start: 89, end: 91, side: "additions" })}>changed lines</button>
    </div>
  ),
}));

import { ReviewWorkbench } from "./ReviewWorkbench";

afterEach(cleanup);

describe("ReviewWorkbench", () => {
  it("opens as a compact file-and-diff tool without editorial navigation", async () => {
    render(<ReviewWorkbench />);

    expect(await screen.findByText("codex-review", { selector: ".compact-bar strong" })).toBeVisible();
    expect(screen.getByRole("navigation", { name: "Changed files" })).toHaveTextContent("src/contracts/review.ts");
    expect(screen.getByRole("region", { name: "Complete diff" })).toBeVisible();
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Review view" })).not.toBeInTheDocument();
  });

  it("keeps file explanation and scoped request beside the diff", async () => {
    render(<ReviewWorkbench />);
    await screen.findByText("codex-review", { selector: ".compact-bar strong" });

    expect(screen.getByLabelText("Active file explanation")).toHaveTextContent("src/contracts/review.ts");
    expect(screen.getByRole("textbox", { name: "Request change" })).toHaveAttribute("placeholder", expect.stringContaining("src/contracts/review.ts"));
  });

  it("maps Pierre line selection to exact feedback scope", async () => {
    render(<ReviewWorkbench />);
    await screen.findByText("codex-review", { selector: ".compact-bar strong" });

    fireEvent.click(screen.getByRole("row", { name: "changed lines" }));

    expect(screen.getByText(/src\/contracts\/review\.ts:89-91/)).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Feedback scope" })).toHaveValue("lines");
  });

  it("renders split diff by default", async () => {
    render(<ReviewWorkbench />);
    expect(await screen.findByRole("table", { name: "Pierre split diff" })).toBeVisible();
  });
});
