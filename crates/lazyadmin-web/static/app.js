// lazyadmin web — vanilla ES module.
// Read-only UI built around the digest. No bundler, no framework.
// Backed by /api/digest, /api/doctor, /api/snapshot, /api/rail,
// /api/header_pip, /api/inspector, /api/views/overview, /api/views/listeners.

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
  listenerTable: null,
  pip: null,
  loadError: null,
  staleSeconds: 0,
  selected: null, // {kind, id}
  showRaw: false,
  route: parseHash(),
  pageFilterText: "",
  sortCol: "port",
  sortDir: "asc",
};

// sync initial URL-backed view state on boot
(function initRouteState() {
  state.pageFilterText = pageFilterTextFromParams(state.route.params);
  const sort = parseSortFromParams(state.route.params);
  state.sortCol = sort.sortCol;
  state.sortDir = sort.sortDir;
})();

// ─── routing ───────────────────────────────────────────────────────
const PAGES = ["overview", "listeners", "workloads", "processes", "doctor", "metrics"];

function parseHash() {
  const raw = location.hash.replace(/^#\/?/, "") || "overview";
  const [path, qs] = raw.split("?");
  const page = PAGES.includes(path) ? path : "overview";
  const params = new URLSearchParams(qs || "");
  return { page, params };
}

function parseSortFromParams(params) {
  const col = params.get("sort") || "port";
  const dir = params.get("dir") || "asc";
  const validCols = ["port", "bind", "exposure", "owner", "project", "confidence", "warnings"];
  return {
    sortCol: validCols.includes(col) ? col : "port",
    sortDir: dir === "desc" ? "desc" : "asc",
  };
}

function pageFilterTextFromParams(params) {
  // q was the original page-local filter parameter. Read it as a
  // compatibility fallback, but write only page_filter so it cannot be
  // confused with global search state.
  return params.get("page_filter") || params.get("q") || "";
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

function setParams(values) {
  const r = state.route;
  Object.entries(values).forEach(([key, value]) => {
    if (value == null || value === "") r.params.delete(key);
    else r.params.set(key, value);
  });
  navigate(r.page, Object.fromEntries(r.params));
}

function nextSortParams(currentSortCol, currentSortDir, clickedColumn) {
  if (currentSortCol === clickedColumn) {
    return { sort: clickedColumn, dir: currentSortDir === "asc" ? "desc" : "asc" };
  }
  return { sort: clickedColumn, dir: "asc" };
}

window.addEventListener("hashchange", () => {
  state.route = parseHash();
  state.pageFilterText = pageFilterTextFromParams(state.route.params);
  const sort = parseSortFromParams(state.route.params);
  state.sortCol = sort.sortCol;
  state.sortDir = sort.sortDir;
  state.listenerTable = null;
  renderPage();
  renderRail();
  loadAll();
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
    const [rail, snap, digest, doctor, pip, listenerTable] = await Promise.all([
      fetchJson("/api/rail"),
      fetchJson("/api/snapshot"),
      fetchJson("/api/digest"),
      fetchJson("/api/doctor"),
      fetchJson("/api/header_pip"),
      fetchJson(listenerTableUrl()),
    ]);
    state.rail = rail;
    state.snapshot = snap;
    state.digest = digest;
    state.doctor = doctor;
    state.pip = pip;
    state.listenerTable = listenerTable;
    state.staleSeconds = pip?.freshness?.age_seconds ?? 0;
    state.loadError = null;
  } catch (err) {
    state.loadError = err.message;
  }
  renderAll();
}

setInterval(loadAll, 5000);
loadAll();

function listenerTableUrl() {
  const params = new URLSearchParams();
  const filter = state.route.params.get("filter") || "all";
  if (filter !== "all") params.set("filter", filter);
  if (state.sortCol !== "port") params.set("sort", state.sortCol);
  if (state.sortDir !== "asc") params.set("dir", state.sortDir);
  const pageFilter = pageFilterTextFromParams(state.route.params);
  if (pageFilter) params.set("page_filter", pageFilter);
  params.set("show_system", "true");
  const qs = params.toString();
  return "/api/views/listeners" + (qs ? "?" + qs : "");
}

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
  attachSortHandlers();
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
  { id: "all", label: "All" },
  { id: "public", label: "Public" },
  { id: "lan", label: "LAN" },
  { id: "conflicts", label: "Conflicts" },
  { id: "orphans", label: "Orphans" },
  { id: "unowned", label: "Unowned" },
  { id: "tracked", label: "Tracked" },
];

function thLabel(col, label) {
  const active = state.sortCol === col;
  const indicator = active ? (state.sortDir === "asc" ? " ▲" : " ▼") : "";
  const ariaSort = active ? (state.sortDir === "asc" ? "ascending" : "descending") : "none";
  return `<th class="sortable ${active ? "sorted" : ""}" data-sort="${col}" scope="col" aria-sort="${ariaSort}"><button type="button" class="sort-button">${esc(label + indicator)}</button></th>`;
}

function renderListeners() {
  const filterId = state.route.params.get("filter") || "all";
  const table = state.listenerTable;
  const rows = table?.rows || [];
  const total = table?.total ?? state.snapshot?.listeners?.length ?? 0;
  const matchCount = table?.returned ?? rows.length;

  const chips = LISTENER_FILTERS.map(
    (f) => `<button class="chip ${f.id === filterId ? "active" : ""}" data-chip="${f.id}">${f.label}</button>`,
  ).join("");

  return `
    <section class="page-head">
      <h1>Listeners</h1>
      <span class="subtle">${matchCount} matched · ${total} total</span>
    </section>
    <div class="chips">${chips}</div>
    ${pageFilterToolbar()}
    <div class="table-wrap">
      <table class="table">
        <thead><tr>
          ${thLabel("port", "Port")}
          ${thLabel("bind", "Bind")}
          ${thLabel("exposure", "Exposure")}
          ${thLabel("owner", "Owner")}
          ${thLabel("project", "Project")}
          ${thLabel("confidence", "Confidence")}
          ${thLabel("warnings", "Warnings")}
        </tr></thead>
        <tbody>${rows.map(listenerTableRow).join("") || emptyRow("no listeners discovered yet", 7)}</tbody>
      </table>
    </div>
  `;
}

function listenerTableRow(row) {
  const expClass = row.signal === "public" ? "exp-public"
    : row.signal === "lan" ? "exp-lan" : "exp-loop";
  const expGlyph = row.signal === "public" ? "●" : row.signal === "lan" ? "◐" : "·";
  const conflictCls = row.is_conflict ? "is-conflict" : "";
  const trackedCls = row.is_tracked ? "is-tracked" : "";
  const projectCls = row.is_project ? "is-project" : "";
  const systemCls = row.is_system ? "is-system" : "";
  const cls = [conflictCls, trackedCls, projectCls, systemCls, isSelected("listener", row.id) ? "selected" : ""]
    .filter(Boolean).join(" ");
  return `<tr class="row ${cls}" data-row data-kind="listener" data-id="${esc(row.id)}">
    <td class="mono port-cell">${row.port ?? "—"}</td>
    <td class="bind-cell mono"><span class="exp-glyph ${expClass}">${expGlyph}</span>${esc(row.endpoint_label || row.bind_label)}<div class="secondary">${esc(row.protocol_label)}</div></td>
    <td>${esc(row.exposure_label)}</td>
    <td>${esc(row.owner_label)}</td>
    <td>${esc(row.project_label || "—")}</td>
    <td>${esc(row.confidence || "")}</td>
    <td>${row.warning_count || ""}</td>
  </tr>`;
}

// ─── workloads ────────────────────────────────────────────────────
function renderWorkloads() {
  const snap = state.snapshot;
  const filtered = applyPageTextFilter(snap.workloads, (w) =>
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
    .join("") || emptyRow("no workloads discovered yet", 4);
  return `
    <section class="page-head"><h1>Workloads</h1>
      <span class="subtle">${filtered.length} matched · ${snap.workloads.length} total</span></section>
    ${pageFilterToolbar()}
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
  const filtered = applyPageTextFilter(snap.processes, (p) =>
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
    ${pageFilterToolbar()}
    <div class="table-wrap">
      <table class="table">
        <thead><tr><th>PID</th><th>User</th><th>Command</th><th>CWD</th></tr></thead>
        <tbody>${groups || emptyRow("no processes", 4)}</tbody>
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
    ? `<div class="value-line warn">drop counter unavailable in stateless run</div>
       <div class="caption">${dropped} event drop(s) were reported without a live 60s denominator. Dropped discovery hints mean the next full snapshot is authoritative.</div>`
    : `<div class="empty">No events dropped in the observable window.</div>
       <div class="caption">Dropped discovery hints mean the next full snapshot is authoritative; increase event capacity only if this keeps rising.</div>`;

  const adapterBlock = snap.managers && snap.managers.length
    ? `<table class="table" style="margin-top:6px">
        <thead><tr><th>Manager</th><th>Available</th><th>Permission</th></tr></thead>
        <tbody>${snap.managers.map((m) =>
          `<tr><td class="mono">${esc(m.name)}</td><td>${m.available ? "yes" : "no"}</td><td>${esc(m.permission)}</td></tr>`,
        ).join("")}</tbody>
      </table>`
    : `<div class="empty">No events in last 60s — adapter is idle (this is normal).</div>
       <div class="caption">Adapter events are refresh hints. Zero events usually means the system is idle, not broken.</div>`;

  return `
    <section class="page-head"><h1>Metrics</h1>
      <span class="subtle">read-only · sourced from /api/snapshot</span></section>
    <div class="metrics-stack">
      <div class="metric-block">
        <h3>Listener exposure histogram</h3>
        <div class="caption">Listener counts show exposure and triage shape; public, conflict, and orphan bars deserve review first.</div>
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

// ─── shared page-local toolbar / filter ──────────────────────────
function pageFilterToolbar() {
  const matchHint = state.pageFilterText.startsWith("~")
    ? "page fuzzy filter"
    : "page substring filter";
  return `<div class="toolbar">
    <label class="page-filter">
      <span class="strategy-hint">${matchHint}</span>
      <input id="pageFilterInput" type="text" placeholder="filter this page (prefix ~ for fuzzy)" value="${esc(state.pageFilterText)}">
    </label>
  </div>`;
}

function applyPageTextFilter(items, hayFn) {
  const q = state.pageFilterText.trim();
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
  const input = $("#pageFilterInput");
  if (input) {
    input.addEventListener("input", () => {
      state.pageFilterText = input.value;
      setParams({ page_filter: input.value, q: null });
      // re-render but keep focus
      renderPage();
      const next = $("#pageFilterInput");
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

function attachSortHandlers() {
  $$(".sortable button").forEach((btn) =>
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      const col = btn.closest("[data-sort]").dataset.sort;
      const next = nextSortParams(state.sortCol, state.sortDir, col);
      setParams(next);
    }),
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
    $$("#inspectorBody [data-jump]").forEach((el) =>
      el.addEventListener("click", () => {
        const target = JSON.parse(el.dataset.jump);
        handleJumpTarget(target);
      }),
    );
  } catch (err) {
    title.textContent = "Inspector";
    body.innerHTML = `<div class="error-banner">Entity gone — refresh<br>${esc(err.message)}</div>`;
  }
}

function renderInspectorView(view) {
  // Per-entity-kind templated layout driven by the typed sections
  // emitted by `InspectorView::to_sections()` server-side. NEVER a
  // <pre>{JSON}</pre>; raw view is debug-only and lives in #rawSlot.
  const sections = inspectorSections(view);
  const titleId = pickIdString(view);
  const titleRow = `
    <div class="insp-title">${esc(view.title || humanKind(view.kind))}</div>
    ${titleId ? `<div class="insp-id mono">${esc(titleId)} <button class="copy-btn" data-value="${esc(titleId)}">copy</button></div>` : ""}
  `;
  const sectionsHtml = sections.map(renderSection).join("");
  return `${titleRow}${sectionsHtml}<div id="rawSlot"></div>`;
}

function inspectorSections(view) {
  // Mirror of `InspectorView::to_sections()` over the JSON shape
  // returned by `/api/inspector`. The server sends typed fields per
  // variant; this function flattens them to the same { heading,
  // rows: [{label, value, secondary?, jump_target?}] } shape the
  // server uses internally.
  switch (view.kind) {
    case "listener": return listenerSections(view);
    case "workload": return workloadSections(view);
    case "process": return processSections(view);
    case "project": return projectSections(view);
    case "manager": return managerSections(view);
    case "tracked_run": return trackedRunSections(view);
    case "warning_group": return warningGroupSections(view);
    default: return [];
  }
}

function renderSection(section) {
  const rows = section.rows.map((r) => {
    const jumpAttr = r.jump_target
      ? ` data-jump='${esc(JSON.stringify(r.jump_target))}'`
      : "";
    const secondary = r.secondary ? `<div class="secondary">${esc(r.secondary)}</div>` : "";
    const cls = r.jump_target ? "insp-row jump" : "insp-row";
    return `<div class="${cls}"${jumpAttr}>
      <div class="label">${esc(r.label)}</div>
      <div class="value mono">${esc(r.value)}${secondary}</div>
    </div>`;
  }).join("");
  return `<section class="insp-section">
    <h3>${esc(section.heading)}</h3>
    <div class="insp-rows">${rows}</div>
  </section>`;
}

function listenerSections(v) {
  const out = [];
  out.push({ heading: "IDENTITY", rows: [
    row("listener id", v.identity.listener_id),
    row("bind", v.identity.bind),
    row("protocol", String(v.identity.protocol)),
    row("family", String(v.identity.family)),
    row("exposure", String(v.identity.exposure)),
    row("state", String(v.identity.state)),
    row("netns", v.identity.netns),
    row("owner", v.identity.owner_label),
    ...(v.identity.user ? [row("user", v.identity.user)] : []),
  ]});
  if (v.process) out.push({ heading: "PROCESS", rows: processFragmentRows(v.process) });
  if (v.related_listeners?.length) out.push({
    heading: "RELATED",
    rows: v.related_listeners.map((r) => ({
      label: String(r.exposure),
      value: `${r.bind} (${r.listener_id})`,
      jump_target: { kind: "listener", id: r.listener_id },
    })),
  });
  if (v.project) out.push({ heading: "PROJECT", rows: [
    { label: "name", value: v.project.name, jump_target: { kind: "project", id: v.project.project_id } },
    row("root", v.project.root),
  ]});
  out.push({ heading: "CONFIDENCE", rows: confidenceRows(v.confidence) });
  if (v.actions?.length) out.push({ heading: "ACTIONS", rows: actionRows(v.actions) });
  if (v.warnings?.length) out.push({ heading: "WARNINGS", rows: warningRows(v.warnings) });
  return out;
}

function workloadSections(v) {
  const out = [];
  const id = v.identity;
  const idRows = [
    row("workload id", id.workload_id),
    row("display name", id.display_name),
    row("runtime", String(id.runtime)),
    row("state", String(id.state)),
    ...(id.health ? [row("health", id.health)] : []),
    ...(id.restart_policy ? [row("restart policy", id.restart_policy)] : []),
  ];
  out.push({ heading: "IDENTITY", rows: idRows });
  if (v.child_processes?.length) {
    const rows = v.child_processes.slice(0, 10).map((p) => ({
      label: `pid ${p.pid}`,
      value: p.cmdline_full,
      jump_target: { kind: "process", id: JSON.stringify(p.key) },
    }));
    if (v.child_processes.length > 10) rows.push(row("more", `+${v.child_processes.length - 10} children`));
    out.push({ heading: "PROCESS", rows });
  }
  if (v.listeners?.length) out.push({
    heading: "RELATED",
    rows: v.listeners.map((r) => ({
      label: String(r.exposure),
      value: `${r.bind} (${r.listener_id})`,
      jump_target: { kind: "listener", id: r.listener_id },
    })),
  });
  if (v.project) out.push({ heading: "PROJECT", rows: [
    { label: "name", value: v.project.name, jump_target: { kind: "project", id: v.project.project_id } },
    row("root", v.project.root),
  ]});
  if (v.manager) out.push({ heading: "MANAGER", rows: [
    { label: "name", value: v.manager.name, jump_target: { kind: "manager", id: v.manager.manager_id } },
    row("kind", String(v.manager.kind)),
  ]});
  out.push({ heading: "CONFIDENCE", rows: confidenceRows(v.confidence) });
  if (v.actions?.length) out.push({ heading: "ACTIONS", rows: actionRows(v.actions) });
  if (v.warnings?.length) out.push({ heading: "WARNINGS", rows: warningRows(v.warnings) });
  return out;
}

function processSections(v) {
  const out = [];
  out.push({ heading: "IDENTITY", rows: processFragmentRows(v.identity) });
  if (v.listeners?.length) out.push({
    heading: "RELATED",
    rows: v.listeners.map((r) => ({
      label: String(r.exposure),
      value: `${r.bind} (${r.listener_id})`,
      jump_target: { kind: "listener", id: r.listener_id },
    })),
  });
  if (v.workload) out.push({ heading: "WORKLOAD", rows: [
    { label: "name", value: v.workload.display_name, jump_target: { kind: "workload", id: v.workload.workload_id } },
    row("runtime", String(v.workload.runtime)),
  ]});
  if (v.tracked_run) out.push({ heading: "TRACKED RUN", rows: [
    { label: "run id", value: v.tracked_run.run_id, jump_target: { kind: "tracked_run", id: v.tracked_run.run_id } },
    ...(v.tracked_run.tag ? [row("tag", v.tracked_run.tag)] : []),
  ]});
  out.push({ heading: "CONFIDENCE", rows: confidenceRows(v.confidence) });
  if (v.actions?.length) out.push({ heading: "ACTIONS", rows: actionRows(v.actions) });
  if (v.warnings?.length) out.push({ heading: "WARNINGS", rows: warningRows(v.warnings) });
  return out;
}

function projectSections(v) {
  const out = [];
  const id = v.identity;
  out.push({ heading: "IDENTITY", rows: [
    row("project id", id.project_id),
    row("name", id.name),
    row("root", id.root),
    ...(id.git_remote ? [row("git remote", id.git_remote)] : []),
    ...(id.package_manager ? [row("package manager", id.package_manager)] : []),
  ]});
  if (v.workloads?.length) out.push({
    heading: "WORKLOADS",
    rows: v.workloads.map((w) => ({
      label: String(w.runtime),
      value: w.display_name,
      jump_target: { kind: "workload", id: w.workload_id },
    })),
  });
  if (v.listeners?.length) out.push({
    heading: "RELATED",
    rows: v.listeners.map((r) => ({
      label: String(r.exposure),
      value: `${r.bind} (${r.listener_id})`,
      jump_target: { kind: "listener", id: r.listener_id },
    })),
  });
  if (v.markers?.length) out.push({
    heading: "MARKERS",
    rows: v.markers.map((m) => row("marker", m)),
  });
  return out;
}

function managerSections(v) {
  const out = [];
  const id = v.identity;
  out.push({ heading: "IDENTITY", rows: [
    row("manager id", id.manager_id),
    row("name", id.name),
    row("kind", String(id.kind)),
    row("scope", String(id.scope)),
    row("available", String(id.available)),
    row("permission", String(id.permission)),
    ...(id.version ? [row("version", id.version)] : []),
    ...(id.socket ? [row("socket", id.socket)] : []),
  ]});
  if (v.managed_workloads?.length) out.push({
    heading: "MANAGED WORKLOADS",
    rows: v.managed_workloads.map((w) => ({
      label: String(w.runtime),
      value: w.display_name,
      jump_target: { kind: "workload", id: w.workload_id },
    })),
  });
  return out;
}

function trackedRunSections(v) {
  const out = [];
  const id = v.identity;
  out.push({ heading: "IDENTITY", rows: [
    row("run id", id.run_id),
    ...(id.tag ? [row("tag", id.tag)] : []),
    row("command", id.command),
    ...(id.cwd ? [row("cwd", id.cwd)] : []),
    row("state", String(id.state)),
  ]});
  if (v.workload) out.push({ heading: "WORKLOAD", rows: [
    { label: "name", value: v.workload.display_name, jump_target: { kind: "workload", id: v.workload.workload_id } },
  ]});
  if (v.actions?.length) out.push({ heading: "ACTIONS", rows: actionRows(v.actions) });
  return out;
}

function warningGroupSections(v) {
  const rows = [
    row("code", v.code),
    row("label", v.label),
    row("severity", String(v.severity)),
    row("tier", String(v.tier)),
    row("count", String(v.count)),
    row("remediation", v.remediation),
  ];
  if (v.sample_entities?.length) {
    rows.push(row("samples", v.sample_entities.map((e) => `${e.kind}: ${typeof e.id === "string" ? e.id : JSON.stringify(e.id)}`).join("\n")));
  }
  return [{ heading: "WARNING GROUP", rows }];
}

function processFragmentRows(p) {
  const out = [
    row("pid", String(p.pid)),
    row("command", p.cmdline_full),
    ...(p.exe ? [row("exe", p.exe)] : []),
    ...(p.cwd ? [row("cwd", p.cwd)] : []),
    ...(p.user ? [row("user", p.user)] : []),
    ...(p.parent_pid ? [row("parent pid", String(p.parent_pid))] : []),
  ];
  if (p.children?.length) {
    const preview = p.children.slice(0, 5).join(", ");
    const value = p.children.length > 5 ? `${preview}, … (+${p.children.length - 5} more)` : preview;
    out.push({ label: "children", value, secondary: `${p.children.length} pids` });
  }
  return out;
}

function confidenceRows(block) {
  const out = [row("value", String(block.value))];
  for (const sig of block.signals || []) {
    out.push({ label: signalLabel(sig.signal), value: sig.claim, secondary: sig.adapter });
  }
  if (!block.signals?.length) {
    out.push({ label: "note", value: "no provenance recorded — confidence is best-effort", secondary: "BestEffort" });
  }
  return out;
}

function signalLabel(sig) {
  return ({
    ProcfsPidInode: "procfs PID→socket inode",
    ContainerInspect: "container runtime inspect",
    CgroupCorrelation: "systemd cgroup correlation",
    ManagerAttribution: "manager attribution heuristic",
    TrackedRunRegistry: "tracked-run registry",
    PortlessRoutes: "portless routes file",
    BestEffort: "best-effort fallback",
  })[sig] || sig;
}

function actionRows(actions) {
  return actions.map((a) => {
    let label = `[${a.key_hint}] ${a.verb}`;
    if (!a.enabled && a.disabled_reason) {
      label += ` — disabled (${a.disabled_reason})`;
    }
    return { label, value: a.command_string };
  });
}

function warningRows(items) {
  return items.map((w) => ({
    label: String(w.severity),
    value: `${w.code}: ${w.message}`,
    jump_target: { kind: "warning_group", id: w.code },
  }));
}

function row(label, value) {
  return { label, value };
}

function pickIdString(view) {
  if (typeof view.id === "string") return view.id;
  if (view.identity?.listener_id) return view.identity.listener_id;
  if (view.identity?.workload_id) return view.identity.workload_id;
  if (view.identity?.project_id) return view.identity.project_id;
  if (view.identity?.manager_id) return view.identity.manager_id;
  if (view.identity?.run_id) return view.identity.run_id;
  if (view.code) return view.code;
  if (view.key) return JSON.stringify(view.key);
  if (view.id) return JSON.stringify(view.id);
  return "";
}

function handleJumpTarget(target) {
  switch (target.kind) {
    case "listener":
      navigate("listeners", { selected: target.id });
      openInspector("listener", target.id);
      break;
    case "workload":
      navigate("workloads", {});
      openInspector("workload", target.id);
      break;
    case "process":
      navigate("processes", {});
      openInspector("process", target.key ? JSON.stringify(target.key) : target.id);
      break;
    case "project":
      navigate("workloads", { project: target.id });
      break;
    case "manager":
      openInspector("manager", target.id);
      break;
    case "tracked_run":
      openInspector("tracked_run", target.id);
      break;
    case "warning_group":
      navigate("doctor", {});
      break;
  }
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

// ─── global search ────────────────────────────────────────────────
const globalSearchState = {
  query: "",
  results: null,
  debounceTimer: null,
  abortController: null,
};

function initGlobalSearch() {
  const input = $("#global-search");
  if (!input) return;
  input.addEventListener("input", onGlobalSearchInput);
  input.addEventListener("keydown", onGlobalSearchKeydown);
  // Auto-focus on load
  input.focus();
}

function onGlobalSearchInput(e) {
  const query = e.target.value;
  globalSearchState.query = query;
  clearTimeout(globalSearchState.debounceTimer);
  if (globalSearchState.abortController) {
    globalSearchState.abortController.abort();
    globalSearchState.abortController = null;
  }
  if (!query.trim()) {
    globalSearchState.results = null;
    renderGlobalSearchResults();
    return;
  }
  globalSearchState.debounceTimer = setTimeout(() => fetchGlobalSearch(query), 120);
}

function onGlobalSearchKeydown(e) {
  if (e.key === "Escape") {
    e.target.value = "";
    globalSearchState.query = "";
    globalSearchState.results = null;
    renderGlobalSearchResults();
    e.target.blur();
  }
}

async function fetchGlobalSearch(query) {
  const controller = new AbortController();
  globalSearchState.abortController = controller;
  try {
    const url = `/api/search?q=${encodeURIComponent(query)}`;
    const res = await fetch(url, {
      headers: { Accept: "application/json" },
      signal: controller.signal,
    });
    if (!res.ok) return;
    const data = await res.json();
    // Only render if the query hasn't changed while we were fetching
    if (globalSearchState.query === query) {
      globalSearchState.results = data;
      renderGlobalSearchResults();
    }
  } catch (err) {
    if (err.name !== "AbortError") {
      console.error("search fetch failed:", err);
    }
  }
}

function renderGlobalSearchResults() {
  const container = $("#globalSearchResults");
  const hint = $("#global-search-hint");
  if (!container) return;

  const results = globalSearchState.results;
  if (!results || !globalSearchState.query.trim()) {
    container.hidden = true;
    container.innerHTML = "";
    if (hint) hint.textContent = "";
    return;
  }

  // Update strategy hint
  if (hint) {
    const total = (results.listeners?.total || 0) + (results.processes?.total || 0)
      + (results.workloads?.total || 0) + (results.projects?.total || 0)
      + (results.managers?.total || 0) + (results.rail_views?.total || 0);
    const parts = [];
    if (results.strategy_hint) parts.push(results.strategy_hint);
    parts.push(`${total} matched`);
    hint.textContent = parts.join(" · ");
  }

  const sections = [];

  if (results.listeners?.hits?.length) {
    sections.push(renderSearchGroup("Listeners", results.listeners, (hit) =>
      `<div class="search-hit" data-search-kind="listener" data-search-id="${esc(hit.id)}">
        <span class="kind-badge">:${hit.port ?? "—"}</span>
        <span class="hit-label">${esc(hit.bind)}<div class="secondary">${esc(hit.owner_label)} · ${esc(String(hit.exposure))}</div></span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (results.processes?.hits?.length) {
    sections.push(renderSearchGroup("Processes", results.processes, (hit) =>
      `<div class="search-hit" data-search-kind="process" data-search-id='${esc(JSON.stringify(hit.key))}'>
        <span class="kind-badge">pid ${hit.pid}</span>
        <span class="hit-label">${esc(hit.exe_or_argv0)}<div class="secondary">${esc(hit.cmdline_compact?.slice(0, 120) || "")}</div></span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (results.workloads?.hits?.length) {
    sections.push(renderSearchGroup("Workloads", results.workloads, (hit) =>
      `<div class="search-hit" data-search-kind="workload" data-search-id="${esc(hit.id)}">
        <span class="kind-badge">${esc(hit.runtime)}</span>
        <span class="hit-label">${esc(hit.display_name)}<div class="secondary">${hit.listener_count} listeners · ${hit.pid_count} pids</div></span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (results.projects?.hits?.length) {
    sections.push(renderSearchGroup("Projects", results.projects, (hit) =>
      `<div class="search-hit" data-search-kind="project" data-search-id="${esc(hit.id)}">
        <span class="kind-badge">proj</span>
        <span class="hit-label">${esc(hit.name)}<div class="secondary">${esc(hit.root)}</div></span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (results.managers?.hits?.length) {
    sections.push(renderSearchGroup("Managers", results.managers, (hit) =>
      `<div class="search-hit" data-search-kind="manager" data-search-id="${esc(hit.id)}">
        <span class="kind-badge">${esc(hit.kind)}</span>
        <span class="hit-label">${esc(hit.name)}<div class="secondary">${esc(hit.scope)} · ${hit.available ? "available" : "unavailable"}</div></span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (results.rail_views?.hits?.length) {
    sections.push(renderSearchGroup("Views", results.rail_views, (hit) =>
      `<div class="search-hit" data-search-kind="rail_view" data-search-id="${esc(hit.id)}">
        <span class="kind-badge">view</span>
        <span class="hit-label">${esc(hit.label)}</span>
        <span class="hit-score">${hit.score}</span>
      </div>`
    ));
  }

  if (sections.length === 0) {
    container.innerHTML = `<div class="search-results-panel"><div class="search-empty">no results for "${esc(globalSearchState.query)}"</div></div>`;
  } else {
    container.innerHTML = `<div class="search-results-panel">${sections.join("")}</div>`;
  }
  container.hidden = false;
  attachSearchResultHandlers();
}

function renderSearchGroup(title, group, rowFn) {
  const countLabel = group.truncated
    ? `${group.returned}/${group.total}`
    : `${group.total}`;
  const rows = group.hits.map(rowFn).join("");
  const truncated = group.truncated
    ? `<div class="search-truncated">… +${group.total - group.returned} more</div>`
    : "";
  return `<div class="search-group">
    <div class="search-group-head"><span>${esc(title)}</span><span class="count">${countLabel}</span></div>
    ${rows}
    ${truncated}
  </div>`;
}

function attachSearchResultHandlers() {
  $$("#globalSearchResults .search-hit").forEach((el) =>
    el.addEventListener("click", () => {
      const kind = el.dataset.searchKind;
      const id = el.dataset.searchId;
      handleSearchHitClick(kind, id);
    }),
  );
}

function handleSearchHitClick(kind, id) {
  // Clear search results after navigation
  globalSearchState.results = null;
  renderGlobalSearchResults();
  switch (kind) {
    case "listener":
      navigate("listeners", { selected: id });
      openInspector("listener", id);
      break;
    case "process":
      navigate("processes", {});
      openInspector("process", id);
      break;
    case "workload":
      navigate("workloads", {});
      openInspector("workload", id);
      break;
    case "project":
      navigate("workloads", { project: id });
      break;
    case "manager":
      openInspector("manager", id);
      break;
    case "rail_view":
      navigate(id);
      break;
  }
}

initGlobalSearch();

// ─── empty rows / palette ─────────────────────────────────────────
function emptyRow(msg, colspan = 6) {
  return `<tr><td colspan="${colspan}" class="empty-affirm">${esc(msg)}</td></tr>`;
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
  if (e.key === "/" && !isTextInput(e.target)) {
    e.preventDefault();
    const searchInput = $("#global-search");
    if (searchInput) searchInput.focus();
  } else if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    openPalette();
  } else if (e.key === "Escape" && $("#palette").open) {
    closePalette();
  }
});
function isTextInput(el) {
  return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}
