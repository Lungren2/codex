import { useEffect, useState } from "react";
import { WorkbenchNav } from "../../components/WorkbenchNav";
import { runUtility, utilityStatus, type UtilityResult } from "../../lib/backend";

type UtilityName = "session-retention" | "daily-digest";
const utilities: Array<{ name: UtilityName; title: string; description: string; caution: string }> = [
  { name: "session-retention", title: "Session retention", description: "Archives inactive sessions, then removes tracked archives after the retention window.", caution: "A live run can delete expired archived sessions." },
  { name: "daily-digest", title: "Daily digest", description: "Builds the scheduled Codex activity digest and writes the configured report output.", caution: "A live run can invoke Codex to prepare a digest." },
];

export function UtilitiesWorkbench() {
  const [results, setResults] = useState<Partial<Record<UtilityName, UtilityResult>>>({});
  const [pending, setPending] = useState<UtilityName | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refresh = async (name: UtilityName) => {
    setPending(name); setError(null);
    try {
      const result = await utilityStatus(name);
      setResults((current) => ({ ...current, [name]: result }));
    }
    catch (cause) { setError(String(cause)); }
    finally { setPending(null); }
  };
  useEffect(() => { void Promise.all(utilities.map(({ name }) => refresh(name))); }, []);
  const run = async (name: UtilityName) => {
    if (!confirm(`Run ${utilities.find((tool) => tool.name === name)?.title} now?\n\n${utilities.find((tool) => tool.name === name)?.caution}`)) return;
    setPending(name); setError(null);
    try {
      const result = await runUtility(name);
      setResults((current) => ({ ...current, [name]: result }));
    }
    catch (cause) { setError(String(cause)); }
    finally { setPending(null); }
  };
  return <main className="utilities-shell">
    <header className="utilities-toolbar">
      <WorkbenchNav active="utilities" />
      <span>Local authority</span>
    </header>
    {error && <output className="utilities-error">{error}</output>}
    <div className="utility-table-wrap">
      <table className="utility-list">
        <caption className="sr-only">Codex maintenance utilities</caption>
        <thead><tr><th scope="col">Utility</th><th scope="col">Local state</th><th scope="col"><span className="sr-only">Actions</span></th></tr></thead>
        <tbody>{utilities.map((tool) => <tr key={tool.name}>
          <th scope="row"><strong>{tool.title}</strong><span>{tool.description}</span><small>{tool.caution}</small></th>
          <td><pre aria-label={`${tool.title} status`}>{results[tool.name] ? JSON.stringify(results[tool.name]?.result, null, 2) : "Checking local state…"}</pre></td>
          <td><div className="utility-actions"><button onClick={() => void refresh(tool.name)} disabled={pending === tool.name}>Refresh</button><button className="utility-run" onClick={() => void run(tool.name)} disabled={pending === tool.name}>{pending === tool.name ? "Working…" : "Run"}</button></div></td>
        </tr>)}</tbody>
      </table>
    </div>
  </main>;
}
