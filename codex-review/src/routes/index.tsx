import { createFileRoute } from "@tanstack/react-router";
import { ReviewWorkbench } from "../features/review/ReviewWorkbench";

export const Route = createFileRoute("/")({
  component: ReviewWorkbench,
});

