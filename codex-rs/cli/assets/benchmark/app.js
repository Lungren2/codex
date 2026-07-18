const state = { benchmarks: [], selectedId: null, detail: null, jobs: [], refreshedJobId: null };
const $ = (selector) => document.querySelector(selector);

const formatNumber = (value) => new Intl.NumberFormat("en-US", { notation: value > 999999 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value || 0);
const formatDuration = (milliseconds) => {
  const seconds = Math.round((milliseconds || 0) / 1000);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
};
const option = (value, label = value) => {
  const element = document.createElement("option");
  element.value = value;
  element.textContent = label;
  return element;
};

async function api(path, options) {
  const response = await fetch(path, { cache: "no-store", ...options });
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || `Request failed: ${response.status}`);
  return payload;
}

async function loadBenchmarks() {
  state.benchmarks = await api("/api/benchmarks");
  if (!state.selectedId || !state.benchmarks.some((item) => item.id === state.selectedId)) state.selectedId = state.benchmarks[0]?.id ?? null;
  renderBenchmarkList();
  if (state.selectedId) await loadDetail();
}

async function loadDetail() {
  state.detail = await api(`/api/benchmarks/${encodeURIComponent(state.selectedId)}`);
  renderDetail();
}

function renderBenchmarkList() {
  $("#suiteCount").textContent = state.benchmarks.length;
  const root = $("#benchmarkList");
  root.replaceChildren(...state.benchmarks.map((benchmark) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `benchmark-item${benchmark.id === state.selectedId ? " selected" : ""}`;
    button.innerHTML = `<strong></strong><span></span>`;
    button.querySelector("strong").textContent = benchmark.id;
    button.querySelector("span").textContent = `${benchmark.runCount} runs · ${benchmark.tasks.length} tasks`;
    button.addEventListener("click", async () => {
      state.selectedId = benchmark.id;
      renderBenchmarkList();
      await loadDetail();
    });
    return button;
  }));
}

function renderDetail() {
  const { summary, runs, activeJob } = state.detail;
  const supports = (control) => summary.supportedControls.includes(control);
  const latestJob = [...state.jobs].reverse().find((job) => job.benchmarkId === summary.id);
  const displayedJob = activeJob ?? latestJob;
  $("#suiteSchema").textContent = `schema ${summary.schemaVersion ?? "?"}`;
  $("#suiteTitle").textContent = summary.id;
  $("#suitePath").textContent = summary.path;
  $("#runCount").textContent = summary.runCount;
  $("#validCount").textContent = summary.validRunCount;
  $("#inputTokens").textContent = formatNumber(summary.inputTokens);
  $("#outputTokens").textContent = formatNumber(summary.outputTokens);
  $("#duration").textContent = formatDuration(summary.durationMs);
  const review = $("#openReview");
  review.hidden = !summary.reviewAvailable;
  review.href = `/review/${encodeURIComponent(summary.id)}/`;
  $("#startRuns").disabled = !summary.runnerAvailable || Boolean(activeJob);

  const stage = $("#stage");
  stage.replaceChildren(option("", summary.stages.length ? "Default stage" : "Not staged"), ...summary.stages.map((id) => option(id)));
  const task = $("#taskId");
  task.replaceChildren(option("", "All tasks"), ...summary.tasks.map((id) => option(id)));
  const parallelism = $("#parallelism");
  parallelism.max = summary.maxParallelism;
  parallelism.disabled = !supports("parallelism");
  if (!summary.supportsParallelism) parallelism.value = 1;
  $("#stageControl").hidden = !supports("stage");
  $("#taskControl").hidden = !supports("task");
  $("#maxNewRunsControl").hidden = !supports("maxNewRuns");
  $("#replicateControl").hidden = !supports("replicate");
  $("#parallelismControl").hidden = !supports("parallelism");
  $("#dryRunControl").hidden = !supports("dryRun");
  $("#rerunInvalidControl").hidden = !supports("rerunInvalid");
  $("#overrideStageGateControl").hidden = !supports("overrideStageGate");
  $("#advancedControls").hidden = !supports("rerunInvalid") && !supports("overrideStageGate");
  $("#jobState").textContent = displayedJob ? `${displayedJob.id} · ${displayedJob.status}` : summary.runnerAvailable ? "ready" : "runner unavailable";
  if (displayedJob?.output) {
    $("#jobOutput").hidden = false;
    $("#jobOutput pre").textContent = displayedJob.output;
  }
  renderRuns(runs);
}

function renderRuns(runs) {
  const query = $("#runFilter").value.trim().toLowerCase();
  const filtered = runs.filter((run) => !query || JSON.stringify(run).toLowerCase().includes(query));
  const body = $("#runsBody");
  body.replaceChildren(...filtered.map((run) => {
    const row = document.createElement("tr");
    const values = [
      run.taskId ?? run.promptId ?? "—",
      run.candidateId ?? run.runId ?? "—",
      run.model ?? "—",
      run.effort ?? "—",
      run.profile ?? "—",
      run.valid === false ? "invalid" : run.valid === true ? "valid" : "recorded",
      formatNumber((run.inputTokens || 0) + (run.outputTokens || 0)),
      formatDuration(run.durationMs),
    ];
    values.forEach((value, index) => {
      const cell = document.createElement("td");
      cell.textContent = value;
      if (index === 5) cell.className = run.valid === false ? "status-invalid" : "status-valid";
      row.append(cell);
    });
    return row;
  }));
  $("#emptyRuns").hidden = filtered.length > 0;
}

async function pollJobs() {
  state.jobs = await api("/api/jobs");
  const latest = [...state.jobs].reverse().find((job) => job.benchmarkId === state.selectedId);
  if (latest) {
    $("#jobState").textContent = `${latest.id} · ${latest.status}`;
    if (latest.output) {
      $("#jobOutput").hidden = false;
      $("#jobOutput pre").textContent = latest.output;
    }
    if (latest.status !== "running" && latest.id !== state.refreshedJobId) {
      state.refreshedJobId = latest.id;
      await loadBenchmarks();
    }
  }
}

$("#runForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = new FormData(event.currentTarget);
  const supports = (control) => state.detail.summary.supportedControls.includes(control);
  const payload = {
    stage: supports("stage") ? form.get("stage") || null : null,
    taskId: supports("task") ? form.get("taskId") || null : null,
    maxNewRuns: supports("maxNewRuns") ? Number(form.get("maxNewRuns")) || null : null,
    replicate: supports("replicate") ? Number(form.get("replicate")) || 1 : null,
    parallelism: supports("parallelism") ? Number(form.get("parallelism")) || 1 : null,
    dryRun: supports("dryRun") && form.get("dryRun") === "on",
    rerunInvalid: supports("rerunInvalid") && form.get("rerunInvalid") === "on",
    overrideStageGate: supports("overrideStageGate") && form.get("overrideStageGate") === "on",
  };
  $("#startRuns").disabled = true;
  $("#jobOutput").hidden = true;
  $("#jobOutput pre").textContent = "";
  try {
    const job = await api(`/api/benchmarks/${encodeURIComponent(state.selectedId)}/run`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    $("#jobState").textContent = `${job.id} · running`;
    await pollJobs();
  } catch (error) {
    $("#jobState").textContent = error.message;
    $("#startRuns").disabled = false;
  }
});
$("#refresh").addEventListener("click", loadBenchmarks);
$("#runFilter").addEventListener("input", () => renderRuns(state.detail?.runs ?? []));

await loadBenchmarks();
setInterval(pollJobs, 2000);
