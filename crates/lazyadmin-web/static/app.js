// lazyadmin web — vanilla ES module.
// Read-only UI built around the digest. No bundler, no framework.
// Backed by /api/digest, /api/doctor, /api/snapshot, /api/rail,
// /api/header_pip, /api/inspector, /api/views/overview.

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );

// ─── state ─────────────────────────────────────────────────────────
const state = {
  rail: [],
  snapshot: null,
  digest: null,
  doctor: null,
  pip: null,
  loadError: null,
  staleSeconds: 0,
  selected: null, // {kind, id}
  showRaw: false,
  route: parseHash(),
  filterText: "",
};

// ─── routing ───────────────────────────────────────────────────────
const PAGES = ["overview", "listeners", "workloads", "processes", "doctor", "metrics"];

function parseHash() {
  const raw = location.hash.replace(/^#\/?/, "") || "overview";
  const [path, qs] = raw.split("?");
  const page = PAGES.includes(path) ? path : "overview";
  const params = new URLSearchParams(qs || "");
  return { page, params };
}

function navigate(page, params = {}) {
  const qs = new URLSearchParams(params);
  const suffix = qs.toString() ? `?${qs}` : "";
  location.hash = `#/${page}${suffix}`;
}

function setParam(key, value) {
  const r = state.route;
  if (value == null || value === "") r.params.delete(key);
  else r.params.set(key, value);
  navigate(r.page, Object.fromEntries(r.params));
}

window.addEventListener("hashchange", () => {
  state.route = parseHash();
  state.filterText = state.route.params.get("q") || "";
  renderPage();
  renderRail();
});

// ─── data loading ─────────────────────────────────────────────────
async function fetchJson(url) {
  const res = await fetch(url, { headers: { Accept: "application/json" } });
  if (!res.ok) {
    let detail;
    try { detail = (await res.json()).message; } catch (_) { detail = res.statusText; }
    throw new Error(`${url}: ${detail}`);
  }
  return res.json();
}

async function loadAll() {
  try {
    const [rail, snap, digest, doctor, pip] = await Promise.all([
      fetchJson("/api/rail"),
      fetchJson("/api/snapshot"),
      fetchJson("/api/digest"),
      fetchJson("/api/doctor"),
      fetchJson("/api/header_pip"),
    ]);
    state.rail = rail;
    state.snapshot = snap;
    state.digest = digest;
    state.doctor = doctor;
    state.pip = pip;
    state.staleSeconds = pip?.freshness?.age_seconds ?? 0;
    state.loadError = null;
  } catch (err) {
    state.loadError = err.message;
  }
  renderAll();
}

setInterval(loadAll, 5000);
loadAll();

// ─── render ────────────────────────────────────────────────────────
function renderAll() {
  renderHeader();
  renderRail();
  renderPage();
}

function renderHeader() {
  const dot = $("#pipDot");
  const label = $("#pipLabel");
  const meta = $("#pipMeta");
  const hostLabel = $("#hostLabel");
  hostLabel.textContent = state.snapshot?.host?.hostname || "localhost";
  if (state.loadError) {
    dot.dataset.state = "error";
    label.textContent = "daemon not reachable";
    meta.textContent = "start with: lazyadmin web";
    return;
  }
  if (!state.pip) {
    dot.dataset.state = "loading";
    label.textContent = "connecting…";
    meta.textContent = "";
    return;
  }
  const adapters = state.pip.adapters;
  const dropped = state.pip.drops?.dropped ?? 0;
  const stale = state.staleSeconds > 5;
  if (stale) {
    dot.dataset.state = "warn";
    label.textContent = `snapshot stale (${state.staleSeconds}s)`;
  } else if (dropped > 0) {
    dot.dataset.state = "warn";
    label.textContent = `events dropped: ${dropped}`;
  } else {
    dot.dataset.state = "ok";
    label.textContent = "healthy";
  }
  meta.textContent =
    `adapters ${adapters.active}/${adapters.total} active · updated ${state.staleSeconds}s ago`;
}

function renderRail() {
  const rail = $("#rail");
  // PLAN-15c #22: subtle Triage / Inventory / Diagnostics grouping.
  const groups = [
    { label: "Triage", ids: ["overview", "doctor"] },
    { label: "Inventory", ids: ["listeners", "workloads", "processes"] },
    { label: "Diagnostics", ids: ["metrics"] },
  ];
  rail.innerHTML = groups
    .map((group) => {
      const buttons = group.ids
        .map((id) => state.rail.find((entry) => entry.id === id))
        .filter(Boolean)
        .map((entry) => {
          const active = entry.id === state.route.page ? "active" : "";
          return `<button data-page="${entry.id}" class="${active}">${esc(entry.label)}</button>`;
        })
        .join("");
      if (!buttons) return "";
      return `<div class="rail-group"><span class="rail-group-label">${group.label}</span><div class="rail-group-buttons">${buttons}</div></div>`;
    })
    .join("");
  $$("#rail button").forEach((b) =>
    b.addEventListener("click", () => navigate(b.dataset.page)),
  );
}

function renderPage() {
  const page = $("#page");
  const banners = renderBanners();
  if (state.loadError && !state.snapshot) {
    page.innerHTML = `${banners}<div class="error-banner">fetch failed: ${esc(state.loadError)}</div>`;
    return;
  }
  if (!state.snapshot) {
    page.innerHTML = `${banners}<div class="empty-affirm">loading snapshot…</div>`;
    return;
  }
  const fns = {
    overview: renderOverview,
    listeners: renderListeners,
    workloads: renderWorkloads,
    processes: renderProcesses,
    doctor: renderDoctor,
    metrics: renderMetrics,
  };
  page.innerHTML = banners + (fns[state.route.page] || renderOverview)();
  attachRowHandlers();
  attachToolbarHandlers();
  attachDigestHandlers();
}

function renderBanners() {
  const out = [];
  if (state.loadError && state.snapshot) {
    out.push(`<div class="error-banner">fetch failed: ${esc(state.loadError)}</div>`);
  }
  if (state.staleSeconds > 5 && state.snapshot) {
    out.push(`<div class="stale-banner">snapshot stale (last update ${state.staleSeconds}s ago)</div>`);
  }
  return out.join("");
}

// ─── overview / digest ────────────────────────────────────────────
function renderOverview() {
  if (!state.digest) {
    return `<section class="page-head"><h1>Overview</h1></section>
      <div class="empty-affirm">digest unavailable in this snapshot.</div>`;
  }
  const d = state.digest;
  const exposedTotal = d.exposed.total_public + d.exposed.total_lan;
  return `
    <section class="page-head">
      <h1>Overview</h1>
      <span class="subtle">${state.snapshot.listeners.length} listeners · ${state.snapshot.processes.length} processes · ${state.snapshot.workloads.length} workloads</span>
    </section>
    <div class="digest-grid">
      ${section({
        title: "Exposed",
        countLabel: `${exposedTotal} total · ${d.exposed.rows.length} shown`,
        empty: d.exposed.rows.length === 0,
        emptyCopy: d.exposed.empty_copy,
        rows: d.exposed.rows.map(exposedRow).join(""),
        viewAll: { page: "listeners", params: { filter: "public" }, label: `view ${exposedTotal} listeners →` },
      })}
      ${section({
        title: "Conflicts",
        countLabel: `${d.conflicts.total} total`,
        empty: d.conflicts.rows.length === 0,
        emptyCopy: d.conflicts.empty_copy,
        rows: d.conflicts.rows.map(conflictRow).join(""),
        viewAll: { page: "listeners", params: { filter: "conflicts" }, label: `view ${d.conflicts.total} conflicts →` },
      })}
      ${section({
        title: "Your projects",
        countLabel: `${d.your_projects.total} total`,
        empty: d.your_projects.rows.length === 0,
        emptyCopy: d.your_projects.empty_copy,
        rows: d.your_projects.rows.map(projectRow).join(""),
        viewAll: { page: "workloads", params: {}, label: "view workloads →" },
        muted: d.your_projects.rows.length === 0,
      })}
      ${section({
        title: "Triage",
        countLabel: `${d.triage.summary.actionable} actionable · ${d.triage.summary.noise_groups} noise groups`,
        empty: d.triage.summary.actionable === 0,
        emptyCopy: d.triage.empty_copy,
        rows: d.triage.summary.actionable > 0
          ? `<p style="margin:6px 0;color:var(--marker-conflict)">Open Doctor to review grouped warnings.</p>`
          : "",
        viewAll: { page: "doctor", params: {}, label: `open Doctor →` },
        muted: d.triage.summary.actionable === 0,
      })}
    </div>
  `;
}

function section({ title, countLabel, empty, emptyCopy, rows, viewAll, muted }) {
  const emptyClass = muted ? "empty-affirm" : "empty-affirm ok";
  const body = empty
    ? `<div class="${emptyClass}">${esc(emptyCopy)}</div>`
    : rows;
  return `<section class="digest-section">
    <h2>${esc(title)}<span class="digest-count">${esc(countLabel)}</span></h2>
    ${body}
    <div class="section-foot">
      <button class="link" data-go data-page="${viewAll.page}" data-params='${JSON.stringify(viewAll.params)}'>${esc(viewAll.label)}</button>
    </div>
  </section>`;
}

function exposedRow(r) {
  const cls = r.exposure === "public" ? "row-public" : "row-lan";
  const glyph = r.unowned ? "○" : "●";
  const sub = [r.owner_label, r.project || "no project", r.extra_ports ? `+${r.extra_ports} more ports` : null]
    .filter(Boolean)
    .join(" · ");
  return `<div class="digest-row ${cls}" data-go data-page="listeners" data-params='{"filter":"public","selected":${JSON.stringify(r.listener_id)}}'>
    <span class="glyph">${glyph}</span>
    <span class="label"><div class="primary">${esc(r.bind)}</div><div class="secondary">${esc(sub)}</div></span>
    <span class="badge">${esc(r.exposure)}</span>
  </div>`;
}

function conflictRow(r) {
  return `<div class="digest-row row-conflict" data-go data-page="listeners" data-params='{"filter":"conflicts","selected":${JSON.stringify(r.listener_id)}}'>
    <span class="glyph">┃</span>
    <span class="label"><div class="primary">${esc(r.bind)}</div><div class="secondary">${esc(r.owner_count)} owners · ${esc(r.reason)}</div></span>
    <span class="badge">${esc(r.severity)}</span>
  </div>`;
}

function projectRow(r) {
  return `<div class="digest-row row-project" data-go data-page="workloads" data-params='{"project":${JSON.stringify(r.project_id)}}'>
    <span class="glyph">▎</span>
    <span class="label"><div class="primary">${esc(r.name)}</div><div class="secondary">${r.listener_count} listeners · ${esc(r.root)}</div></span>
    <span class="badge">${r.workload_count} wl</span>
  </div>`;
}

// ─── listeners ─────────────────────────────────────────────────────
const LISTENER_FILTERS = [
  { id: "all", label: "All", match: () => true },
  { id: "public", label: "Public", match: (l) => l.exposure === "public" || l.exposure === "lan_or_public" },
  { id: "lan", label: "LAN", match: (l) => l.exposure === "lan_or_public" },
  { id: "conflicts", label: "Conflicts", match: (l, snap) =>
      snap.warnings.some((w) => w.code === "CONFLICT" && w.entity?.id === l.id) ||
      (l.owners || []).length > 1 },
  { id: "orphans", label: "Orphans", match: (l) => (l.owners || []).length === 0 },
  { id: "unowned", label: "Unowned", match: (l) => (l.owners || []).length === 0 },
  { id: "tracked", label: "Tracked", match: (l, snap) =>
      (l.owners || []).some((o) => o.kind === "run") ||
      snap.workloads.some((w) => w.lazyadmin_run_id && (w.listeners || []).includes(l.id)) },
];

function renderListeners() {
  const snap = state.snapshot;
  const filterId = state.route.params.get("filter") || "all";
  const active = LISTENER_FILTERS.find((f) => f.id === filterId) || LISTENER_FILTERS[0];
  const all = snap.listeners.slice();
  const matched = all.filter((l) => active.match(l, snap));
  const filtered = applyTextFilter(matched, listenerHaystack);
  const total = all.length;
  const matchCount = filtered.length;

  const chips = LISTENER_FILTERS.map(
    (f) => `<button class="chip ${f.id === filterId ? "active" : ""}" data-chip="${f.id}">${f.label}</button>`,
  ).join("");

  return `
    <section class="page-head">
      <h1>Listeners</h1>
      <span class="subtle">${matchCount} matched · ${total} total</span>
    </section>
    <div class="chips">${chips}</div>
    ${searchToolbar()}
    <div class="table-wrap">
      <table class="table">
        <thead><tr>
          <th>Bind</th><th>Exposure</th><th>Owner</th><th>Project</th><th>Confidence</th><th>Warnings</th>
        </tr></thead>
        <tbody>${filtered.map(listenerTableRow).join("") || emptyRow("no listeners discovered yet")}</tbody>
      </table>
    </div>
  `;
}

function listenerHaystack(l) {
  return [
    l.id, l.bind_addr, l.port, l.protocol, l.exposure, l.path,
    ...(l.owners || []).map((o) => o.id?.pid ?? o.id ?? ""),
  ].filter((x) => x != null).map(String).join(" ").toLowerCase();
}

function listenerTableRow(l) {
  const exposure = l.exposure || "loopback";
  const expClass = exposure === "public" ? "exp-public"
    : exposure === "lan_or_public" ? "exp-lan" : "exp-loop";
  const expGlyph = exposure === "public" ? "●" : exposure === "lan_or_public" ? "●" : "·";
  const bind = l.path || `${l.bind_addr || "*"}:${l.port ?? "?"}`;
  const owner = ownerLabel(l);
  const project = projectFor(l);
  const warnings = (state.snapshot.warnings || []).filter(
    (w) => w.entity?.kind === "listener" && w.entity.id === l.id,
  );
  const conflictCls = warnings.some((w) => w.code === "CONFLICT") || (l.owners || []).length > 1
    ? "is-conflict" : "";
  const trackedCls = (l.owners || []).some((o) => o.kind === "run") ? "is-tracked" : "";
  const projectCls = project ? "is-project" : "";
  const cls = [conflictCls, trackedCls, projectCls, isSelected("listener", l.id) ? "selected" : ""]
    .filter(Boolean).join(" ");
  return `<tr class="row ${cls}" data-row data-kind="listener" data-id="${esc(l.id)}">
    <td class="bind-cell mono"><span class="exp-glyph ${expClass}">${expGlyph}</span>${esc(bind)}<div class="secondary">${esc(l.protocol)}</div></td>
    <td>${esc(exposure)}</td>
    <td>${esc(owner)}</td>
    <td>${esc(project || "—")}</td>
    <td>${esc(l.confidence || "")}</td>
    <td>${warnings.length || ""}</td>
  </tr>`;
}

function ownerLabel(l) {
  const o = (l.owners || [])[0];
  if (!o) return "—";
  if (o.kind === "process") return `pid ${o.id?.pid ?? "?"}`;
  if (o.kind === "workload") {
    const wl = state.snapshot.workloads.find((w) => w.id === o.id);
    return wl?.display_name || `workload ${o.id}`;
  }
  return `${o.kind} ${o.id}`;
}

function projectFor(l) {
  for (const o of l.owners || []) {
    if (o.kind === "workload") {
      const wl = state.snapshot.workloads.find((w) => w.id === o.id);
      if (wl?.project) {
        const p = state.snapshot.projects.find((p) => p.id === wl.project);
        return p?.name || wl.project;
      }
    }
  }
  return null;
}

// ─── workloads ────────────────────────────────────────────────────
function renderWorkloads() {
  const snap = state.snapshot;
  const filtered = applyTextFilter(snap.workloads, (w) =>
    [w.display_name, w.runtime, w.state, w.id].join(" ").toLowerCase(),
  );
  const groups = new Map();
  for (const w of filtered) {
    const key = w.manager
      ? snap.managers.find((m) => m.id === w.manager)?.name || `manager ${w.manager}`
      : `runtime: ${w.runtime}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(w);
  }
  const rows = Array.from(groups.entries())
    .map(([head, items]) => `
      <tr class="group-head"><td colspan="4">${esc(head)} · ${items.length}</td></tr>
      ${items.map(workloadRow).join("")}
    `)
    .join("") || emptyRow("no workloads discovered yet");
  return `
    <section class="page-head"><h1>Workloads</h1>
      <span class="subtle">${filtered.length} matched · ${snap.workloads.length} total</span></section>
    ${searchToolbar()}
    <div class="table-wrap">
      <table class="table">
        <thead><tr><th>Name</th><th>State</th><th>Health</th><th>Listeners</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
  `;
}

function workloadRow(w) {
  const cls = isSelected("workload", w.id) ? "selected" : "";
  return `<tr class="row ${cls}" data-row data-kind="workload" data-id="${esc(w.id)}">
    <td class="mono">${esc(w.display_name || w.id)}</td>
    <td>${esc(w.state)}</td>
    <td>${esc(w.health || "—")}</td>
    <td>${(w.listeners || []).length}</td>
  </tr>`;
}

// ─── processes ────────────────────────────────────────────────────
function renderProcesses() {
  const snap = state.snapshot;
  const filtered = applyTextFilter(snap.processes, (p) =>
    [p.pid, p.exe, p.user, (p.cmdline || p.command || []).join(" ")].join(" ").toLowerCase(),
  );
  const byParent = new Map();
  for (const p of filtered) {
    const key = p.ppid || 0;
    if (!byParent.has(key)) byParent.set(key, []);
    byParent.get(key).push(p);
  }
  const groups = Array.from(byParent.entries())
    .sort((a, b) => a[0] - b[0])
    .map(([ppid, items]) => `
      <tr class="group-head"><td colspan="4">parent pid ${esc(ppid || "—")} · ${items.length}</td></tr>
      ${items.slice(0, 200).map(processRow).join("")}
    `).join("");
  return `
    <section class="page-head"><h1>Processes</h1>
      <span class="subtle">${filtered.length} matched · ${snap.processes.length} total</span></section>
    ${searchToolbar()}
    <div class="table-wrap">
      <table class="table">
        <thead><tr><th>PID</th><th>User</th><th>Command</th><th>CWD</th></tr></thead>
        <tbody>${groups || emptyRow("no processes")}</tbody>
      </table>
    </div>
  `;
}

function processRow(p) {
  const cmd = (p.cmdline || p.command || []).join(" ") || p.exe || "";
  const id = JSON.stringify(p.key);
  const cls = isSelected("process", id) ? "selected" : "";
  return `<tr class="row ${cls}" data-row data-kind="process" data-id='${esc(id)}'>
    <td class="mono">${esc(p.pid)}</td>
    <td>${esc(p.user || "—")}</td>
    <td class="mono">${esc(cmd.slice(0, 160))}</td>
    <td class="mono">${esc(p.cwd || "")}</td>
  </tr>`;
}

// ─── doctor ───────────────────────────────────────────────────────
function renderDoctor() {
  const dv = state.doctor;
  if (!dv || dv.groups.length === 0) {
    return `<section class="page-head"><h1>Doctor</h1></section>
      <div class="empty-affirm ok">Everything's clean.</div>`;
  }
  const summary =
    `${dv.actionable_count} actionable · ${dv.noise_group_count} noise groups (${dv.noise_total_count} total)`;
  const body = dv.groups.map((g) => {
    const tier = `tier-${g.tier.toLowerCase()}`;
    const samples = g.sample_entities
      .map((e) => `<span class="sample">${esc(e.kind)}: ${esc(JSON.stringify(e.id))}</span>`)
      .join("");
    return `<details class="doctor-group ${tier}" ${g.expanded ? "open" : ""}>
      <summary>
        <span class="severity-pill">${esc(g.severity)}</span>
        <span><span class="group-title">${esc(g.label)}</span> <span class="group-sub">${esc(g.code)}</span></span>
        <span class="count">${g.count}</span>
      </summary>
      <div class="doctor-group-body">
        <div class="remediation">${esc(g.remediation)}</div>
        ${samples ? `<div class="samples">${samples}</div>` : ""}
      </div>
    </details>`;
  }).join("");
  return `
    <section class="page-head"><h1>Doctor</h1><span class="subtle">${summary}</span></section>
    ${body}
  `;
}

// ─── metrics ──────────────────────────────────────────────────────
function renderMetrics() {
  const snap = state.snapshot;
  const totalListeners = snap.listeners.length;
  const counts = {
    Listeners: totalListeners,
    Public: snap.listeners.filter((l) => l.exposure === "public").length,
    LAN: snap.listeners.filter((l) => l.exposure === "lan_or_public").length,
    Loopback: snap.listeners.filter(
      (l) => l.exposure === "loopback" || l.exposure === "unix_local",
    ).length,
    Conflicts: snap.warnings.filter((w) => w.code === "CONFLICT").length,
    Orphans: snap.listeners.filter((l) => (l.owners || []).length === 0).length,
  };
  const max = Math.max(1, ...Object.values(counts));
  const histRows = Object.entries(counts).map(([k, v]) => {
    const cls = k === "Public" ? "row-public" : k === "Conflicts" ? "row-conflict" : k === "Orphans" ? "row-orphan" : "";
    const pct = (v / max) * 100;
    return `<div class="histogram-row ${cls}">
      <span class="label">${k}</span>
      <span class="bar"><span class="fill" style="width:${pct.toFixed(1)}%"></span></span>
      <span class="num">${v}</span>
    </div>`;
  }).join("");

  const dropped = snap.metadata?.events_dropped ?? 0;
  const eventsBlock = dropped > 0
    ? `<div class="value-line"><span class="accent">${dropped}</span> events dropped since startup</div>
       <div class="caption">If this number climbs steadily, raise <code>events.queue_capacity</code> in your config.</div>`
    : `<div class="empty">No events dropped — fan-in is keeping up.</div>`;

  const adapterBlock = snap.managers && snap.managers.length
    ? `<table class="table" style="margin-top:6px">
        <thead><tr><th>Manager</th><th>Available</th><th>Permission</th></tr></thead>
        <tbody>${snap.managers.map((m) =>
          `<tr><td class="mono">${esc(m.name)}</td><td>${m.available ? "yes" : "no"}</td><td>${esc(m.permission)}</td></tr>`,
        ).join("")}</tbody>
      </table>`
    : `<div class="empty">No managers reachable in this snapshot.</div>`;

  return `
    <section class="page-head"><h1>Metrics</h1>
      <span class="subtle">read-only · sourced from /api/snapshot</span></section>
    <div class="metrics-stack">
      <div class="metric-block">
        <h3>Listener exposure histogram</h3>
        <div class="caption">Counts of listeners broken out by exposure tier and warning class.</div>
        <div class="histogram">${histRows}</div>
      </div>
      <div class="metric-block">
        <h3>Events dropped</h3>
        ${eventsBlock}
      </div>
      <div class="metric-block">
        <h3>Discovery health</h3>
        <div class="caption">Adapter-side health for the managers backing this snapshot.</div>
        ${adapterBlock}
      </div>
    </div>
  `;
}

// ─── shared toolbar / filter ──────────────────────────────────────
function searchToolbar() {
  const matchHint = state.filterText.startsWith("~")
    ? "fuzzy match"
    : "substring match";
  return `<div class="toolbar">
    <label class="search">
      <span class="strategy-hint">${matchHint}</span>
      <input id="filterInput" type="text" placeholder="filter (prefix ~ for fuzzy)" value="${esc(state.filterText)}">
    </label>
  </div>`;
}

function applyTextFilter(items, hayFn) {
  const q = state.filterText.trim();
  if (!q) return items;
  if (q.startsWith("~")) {
    const needle = q.slice(1).toLowerCase();
    return items.filter((x) => fuzzyMatch(hayFn(x), needle));
  }
  const needle = q.toLowerCase();
  return items.filter((x) => hayFn(x).includes(needle));
}

function fuzzyMatch(haystack, needle) {
  let i = 0;
  for (const ch of haystack) {
    if (ch === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return needle === "";
}

function attachToolbarHandlers() {
  const input = $("#filterInput");
  if (input) {
    input.addEventListener("input", () => {
      state.filterText = input.value;
      setParam("q", input.value);
      // re-render but keep focus
      renderPage();
      const next = $("#filterInput");
      if (next) {
        next.focus();
        next.setSelectionRange(input.value.length, input.value.length);
      }
    });
  }
  $$(".chip").forEach((c) =>
    c.addEventListener("click", () => setParam("filter", c.dataset.chip)),
  );
}

function attachDigestHandlers() {
  $$("[data-go]").forEach((el) =>
    el.addEventListener("click", () => {
      const params = JSON.parse(el.dataset.params || "{}");
      navigate(el.dataset.page, params);
    }),
  );
}

// ─── inspector ────────────────────────────────────────────────────
function attachRowHandlers() {
  $$("[data-row]").forEach((el) =>
    el.addEventListener("click", () => {
      const kind = el.dataset.kind;
      const id = el.dataset.id;
      state.selected = { kind, id };
      setParam("selected", id);
      openInspector(kind, id);
      $$("[data-row]").forEach((r) => r.classList.toggle(
        "selected",
        r.dataset.kind === kind && r.dataset.id === id,
      ));
    }),
  );
  // Auto-open from URL on page load
  const sel = state.route.params.get("selected");
  if (sel && state.route.page === "listeners") openInspector("listener", sel);
}

function isSelected(kind, id) {
  return state.selected && state.selected.kind === kind && state.selected.id === id;
}

async function openInspector(kind, id) {
  const aside = $("#inspector");
  const body = $("#inspectorBody");
  const title = $("#inspectorTitle");
  document.body.classList.add("with-inspector");
  aside.hidden = false;
  body.innerHTML = `<div class="empty-affirm">loading…</div>`;
  try {
    const view = await fetchJson(`/api/inspector?kind=${encodeURIComponent(kind)}&id=${encodeURIComponent(id)}`);
    title.textContent = `${humanKind(view.kind)} · ${view.title || ""}`.trim();
    body.innerHTML = renderInspectorView(view);
    if (state.showRaw) {
      attachRaw(kind, id);
    }
    $$("#inspectorBody .copy-btn").forEach((btn) =>
      btn.addEventListener("click", () => navigator.clipboard?.writeText(btn.dataset.value || "")),
    );
  } catch (err) {
    title.textContent = "Inspector";
    body.innerHTML = `<div class="error-banner">Entity gone — refresh<br>${esc(err.message)}</div>`;
  }
}

function renderInspectorView(view) {
  // Per-entity-kind templated layout. NEVER a <pre>{JSON}</pre>.
  const facts = (view.facts || [])
    .map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`)
    .join("");
  const idValue = view.id ? JSON.stringify(view.id) : "";
  const idChip = idValue
    ? `<dt>id</dt><dd>${esc(typeof view.id === "string" ? view.id : idValue)} <button class="copy-btn" data-value="${esc(typeof view.id === "string" ? view.id : idValue)}">copy</button></dd>`
    : "";
  const actions = renderInspectorActions(view);
  return `
    <div class="insp-title">${esc(view.title || view.kind)}</div>
    <dl class="fact-list">${idChip}${facts}</dl>
    ${actions}
    <div id="rawSlot"></div>
  `;
}

function renderInspectorActions(view) {
  // Read-only Web UI: actions show the command they WOULD run.
  const commands = {
    listener: ["lazyadmin free <port>", "lazyadmin pause <listener>"],
    workload: ["lazyadmin restart <workload>", "lazyadmin pause <workload>"],
    process: ["kill <pid>", "lazyadmin logs <pid>"],
    project: [],
    manager: [],
    tracked_run: ["lazyadmin logs --run <id>"],
    warning_group: [],
  };
  const list = commands[view.kind] || [];
  if (!list.length) return "";
  return `<div class="actions-block">
    <h3>Actions (preview)</h3>
    ${list.map((cmd) => `<div class="action-stub">
      <code>${esc(cmd)}</code>
      <button disabled title="Web UI is read-only — run this in the TUI or on the CLI">disabled</button>
    </div>`).join("")}
  </div>`;
}

function attachRaw(kind, id) {
  // Pull the matching slice out of the cached snapshot. Show-raw is debug-only.
  const slot = $("#rawSlot");
  if (!slot) return;
  const fragment = pickSnapshotFragment(kind, id);
  if (!fragment) return;
  slot.innerHTML = `<div class="raw-block">${esc(JSON.stringify(fragment, null, 2))}</div>`;
}

function pickSnapshotFragment(kind, id) {
  const snap = state.snapshot;
  if (!snap) return null;
  switch (kind) {
    case "listener": return snap.listeners.find((l) => l.id === id);
    case "workload": return snap.workloads.find((w) => w.id === id);
    case "process": {
      try {
        const key = JSON.parse(id);
        return snap.processes.find((p) =>
          p.key && p.key.pid === key.pid && p.key.boot_id === key.boot_id,
        );
      } catch (_) {
        return snap.processes.find((p) => String(p.pid) === id);
      }
    }
    case "project": return snap.projects.find((p) => p.id === id);
    case "manager": return snap.managers.find((m) => m.id === id);
    case "run":
    case "tracked_run": return snap.tracked_runs.find((r) => r.id === id);
    default: return null;
  }
}

function humanKind(k) {
  return ({
    listener: "Listener",
    workload: "Workload",
    process: "Process",
    project: "Project",
    manager: "Manager",
    tracked_run: "Tracked run",
    warning_group: "Warning group",
  })[k] || k;
}

$("#inspectorClose").addEventListener("click", () => {
  $("#inspector").hidden = true;
  document.body.classList.remove("with-inspector");
  state.selected = null;
  setParam("selected", "");
});
$("#rawToggle").addEventListener("change", (e) => {
  state.showRaw = e.target.checked;
  if (state.selected) openInspector(state.selected.kind, state.selected.id);
});

// ─── empty rows / palette ─────────────────────────────────────────
function emptyRow(msg) {
  return `<tr><td colspan="6" class="empty-affirm">${esc(msg)}</td></tr>`;
}

function openPalette() {
  const dlg = $("#palette");
  if (!dlg.open) dlg.showModal();
  $("#paletteInput").value = "";
  renderPaletteResults("");
  $("#paletteInput").focus();
}
function closePalette() { $("#palette").close(); }

function renderPaletteResults(query) {
  const ul = $("#paletteResults");
  const items = paletteItems().filter((it) =>
    !query || it.label.toLowerCase().includes(query.toLowerCase()),
  ).slice(0, 30);
  ul.innerHTML = items.map((it, idx) =>
    `<li data-idx="${idx}" class="${idx === 0 ? "active" : ""}"><span>${esc(it.label)}</span><span class="meta">${esc(it.meta)}</span></li>`,
  ).join("") || `<li><span class="meta">no matches</span></li>`;
  ul._items = items;
  ul.querySelectorAll("li").forEach((li) =>
    li.addEventListener("click", () => activatePaletteItem(parseInt(li.dataset.idx, 10))),
  );
}

function activatePaletteItem(idx) {
  const it = ($("#paletteResults")._items || [])[idx];
  if (!it) return;
  closePalette();
  it.action();
}

function paletteItems() {
  const items = [];
  for (const r of state.rail) items.push({
    label: `Go to ${r.label}`,
    meta: r.id,
    action: () => navigate(r.id),
  });
  for (const l of state.snapshot?.listeners || []) {
    const bind = l.path || `${l.bind_addr || "*"}:${l.port ?? "?"}`;
    items.push({
      label: `Listener ${bind}`,
      meta: l.exposure,
      action: () => navigate("listeners", { selected: l.id }),
    });
  }
  for (const p of state.snapshot?.projects || []) items.push({
    label: `Project ${p.name}`,
    meta: p.root,
    action: () => navigate("workloads", { project: p.id }),
  });
  for (const code of new Set((state.doctor?.groups || []).map((g) => g.code))) items.push({
    label: `Warning ${code}`,
    meta: "doctor",
    action: () => navigate("doctor"),
  });
  return items;
}

$("#paletteTrigger").addEventListener("click", openPalette);
$("#paletteForm").addEventListener("submit", (e) => {
  e.preventDefault();
  activatePaletteItem(0);
});
$("#paletteInput").addEventListener("input", (e) => renderPaletteResults(e.target.value));
window.addEventListener("keydown", (e) => {
  if ((e.key === "/" || (e.key === "k" && (e.metaKey || e.ctrlKey))) && !isTextInput(e.target)) {
    e.preventDefault();
    openPalette();
  } else if (e.key === "Escape" && $("#palette").open) {
    closePalette();
  }
});
function isTextInput(el) {
  return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}
