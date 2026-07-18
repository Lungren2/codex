import { createFileRoute } from "@tanstack/react-router";
import { SkillsInstaller } from "../features/skills/SkillsInstaller";
export const Route = createFileRoute("/skills")({ component: SkillsInstaller });
