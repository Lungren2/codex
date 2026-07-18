type WorkbenchRoute = "review" | "utilities" | "skills";

const routes: Array<{ id: WorkbenchRoute; href: string; label: string }> = [
  { id: "review", href: "/", label: "Review" },
  { id: "utilities", href: "/utilities", label: "Utilities" },
  { id: "skills", href: "/skills", label: "Skills" },
];

export function WorkbenchNav({ active }: { active: WorkbenchRoute }) {
  return (
    <nav className="workbench-nav" aria-label="Workbench">
      {routes.map((route) => route.id === active
        ? <strong key={route.id} aria-current="page">{route.label}</strong>
        : <a key={route.id} href={route.href}>{route.label}</a>)}
    </nav>
  );
}
