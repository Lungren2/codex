import { createFileRoute } from "@tanstack/react-router";
import { UtilitiesWorkbench } from "../features/utilities/UtilitiesWorkbench";

export const Route = createFileRoute("/utilities")({ component: UtilitiesWorkbench });
