function asObject(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function text(value, fallback = "") {
  return typeof value === "string" && value.trim() ? value.trim() : fallback;
}

function number(value, fallback = null) {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function escapeHtml(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function getPath(source, path) {
  if (!path || typeof path !== "string") return undefined;
  const direct = path.split(".").reduce((current, part) => {
    if (current == null) return undefined;
    const key = part.trim();
    if (!key) return undefined;
    if (/^\d+$/.test(key)) return current[Number(key)];
    return current[key];
  }, source);
  if (direct !== undefined) return direct;

  const leaf = path
    .split(".")
    .map((part) => part.trim())
    .filter(Boolean)
    .pop();
  return leaf ? findByLeafKey(source, leaf) : undefined;
}

function findByLeafKey(source, leaf, depth = 0) {
  if (!source || depth > 5) return undefined;
  if (Array.isArray(source)) {
    for (const item of source) {
      const found = findByLeafKey(item, leaf, depth + 1);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  if (typeof source !== "object") return undefined;
  for (const [key, value] of Object.entries(source)) {
    if (key === leaf) return value;
  }
  for (const value of Object.values(source)) {
    const found = findByLeafKey(value, leaf, depth + 1);
    if (found !== undefined) return found;
  }
  return undefined;
}

function formatScalar(value, binding = {}) {
  const decimals = number(binding.decimals);
  const formatted =
    decimals != null && typeof value === "number"
      ? value.toFixed(Math.max(0, Math.min(4, decimals)))
      : String(value);
  return `${binding.prefix || ""}${formatted}${binding.suffix || binding.unit || ""}`;
}

function formatBoundValue(value, data) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const binding = asObject(value);
    const rawPath = binding.path || binding.from || binding.valueFrom || binding.key;
    const raw = rawPath ? getPath(data, rawPath) : binding.value;
    const fallback = binding.fallback ?? "--";
    const resolved = raw == null || raw === "" ? fallback : raw;
    if (Array.isArray(resolved)) {
      const shown = resolved.slice(0, 5).map((item) => formatScalar(item, binding));
      return shown.length ? shown.join(", ") : String(fallback);
    }
    return formatScalar(resolved, binding);
  }
  return value == null || value === "" ? "--" : String(value);
}

function labelize(value) {
  const label = String(value ?? "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim();
  if (!label) return "Details";
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function firstText(...values) {
  for (const value of values) {
    const resolved = text(value);
    if (resolved) return resolved;
  }
  return "";
}

function appViews(spec) {
  return asArray(spec.views || spec.tabs).filter((view) => view && typeof view === "object");
}

function viewKey(view, index) {
  return firstText(view.id, view.key, view.value, view.label, view.title) || `view-${index}`;
}

function selectedView(spec, state) {
  const views = appViews(spec);
  if (!views.length) return null;
  const requested = firstText(state.activeView, spec.activeView, spec.defaultView, spec.initialView);
  const index = requested
    ? views.findIndex((view, idx) => viewKey(view, idx) === requested)
    : 0;
  const selectedIndex = index >= 0 ? index : 0;
  const selected = views[selectedIndex];
  state.activeView = viewKey(selected, selectedIndex);
  return selected;
}

function viewSpec(base, view) {
  return {
    ...base,
    ...view,
    title: firstText(view.title, view.label, base.title),
    subtitle: firstText(view.subtitle, view.summary, view.description, base.subtitle, base.summary),
    content: view.content ?? view.body ?? view.description ?? base.content ?? base.body,
    metrics: view.metrics ?? base.metrics,
    sections: view.sections ?? base.sections,
    items: view.items ?? base.items,
    rows: view.rows ?? base.rows,
    actions: view.actions ?? base.actions,
    accent: view.accent ?? asObject(view.visual).accent ?? base.accent,
  };
}

function renderViewTabs(views, state) {
  if (views.length <= 1) return "";
  const active = text(state.activeView);
  return `<div class="app-shell-tabs" role="tablist">${views
    .map((view, index) => {
      const key = viewKey(view, index);
      const label = firstText(view.label, view.title, key);
      const selected = key === active;
      return `<button type="button" class="app-shell-tab${selected ? " is-active" : ""}" role="tab" aria-selected="${selected ? "true" : "false"}" data-app-view="${escapeHtml(key)}">${escapeHtml(label)}</button>`;
    })
    .join("")}</div>`;
}

function checklistKey(namespace, item, index) {
  const value = asObject(item);
  return `${namespace}:${firstText(value.id, value.key, value.label, value.name, value.title) || index}`;
}

function checklistChecked(state, key, fallback) {
  if (!state.checked || typeof state.checked !== "object") state.checked = {};
  if (Object.prototype.hasOwnProperty.call(state.checked, key)) return Boolean(state.checked[key]);
  return Boolean(fallback);
}

function renderChecklist(items, data, state, namespace) {
  const rows = asArray(items);
  if (!rows.length) return "";
  return `<ul class="app-shell-checklist">${rows
    .map((row, index) => {
      const item = asObject(row);
      const key = checklistKey(namespace, item, index);
      const checked = checklistChecked(state, key, item.done || item.checked || item.complete);
      const label = formatBoundValue(firstText(item.label, item.name, item.title) || row, data);
      const detail = item.detail || item.value || item.summary || item.reps || item.duration || "";
      return `<li>
        <button type="button" class="app-shell-check${checked ? " is-checked" : ""}" data-app-check="${escapeHtml(key)}" aria-pressed="${checked ? "true" : "false"}">
          <span class="app-shell-check-box">${checked ? "&#10003;" : ""}</span>
          <span class="app-shell-check-copy">
            <span>${escapeHtml(label)}</span>
            ${detail ? `<strong>${escapeHtml(formatBoundValue(detail, data))}</strong>` : ""}
          </span>
        </button>
      </li>`;
    })
    .join("")}</ul>`;
}

function renderContentBlock(content, data) {
  const raw = formatBoundValue(content, data).trim();
  if (!raw || raw === "--") return "";
  const lines = raw.split(/\r?\n/).map((line) => line.trim());
  let html = "";
  let list = [];
  const flushList = () => {
    if (!list.length) return;
    html += `<ul>${list.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>`;
    list = [];
  };

  for (const line of lines) {
    if (!line) {
      flushList();
      continue;
    }
    if (line.startsWith("- ")) {
      list.push(line.slice(2).trim());
      continue;
    }
    flushList();
    const heading = line.match(/^#{1,3}\s+(.+)$/);
    if (heading) {
      html += `<h3>${escapeHtml(heading[1])}</h3>`;
    } else {
      html += `<p>${escapeHtml(line)}</p>`;
    }
  }
  flushList();
  return html ? `<section class="app-shell-content">${html}</section>` : "";
}

function metricValue(metric) {
  const item = asObject(metric);
  const path = item.path || item.from || item.valueFrom || item.key;
  if (item.value !== undefined) return item.value;
  if (path) {
    return {
      path,
      fallback: item.fallback,
      prefix: item.prefix,
      suffix: item.suffix,
      unit: item.unit,
      decimals: item.decimals,
    };
  }
  return "--";
}

function renderMetric(metric, data) {
  const item = asObject(metric);
  const label = text(item.label || item.name, "Value");
  const detail = item.detail ?? item.description ?? "";
  return `<div class="app-shell-metric">
    <span>${escapeHtml(label)}</span>
    <strong>${escapeHtml(formatBoundValue(metricValue(item), data))}</strong>
    ${detail ? `<em>${escapeHtml(formatBoundValue(detail, data))}</em>` : ""}
  </div>`;
}

function renderRows(rows, data) {
  return rows
    .map((row) => {
      if (row && typeof row === "object" && !Array.isArray(row)) {
        const value = asObject(row);
        const entries = Object.entries(value).filter(([, entryValue]) => {
          return entryValue != null && entryValue !== "";
        });
        const label =
          value.label ??
          value.name ??
          value.title ??
          (entries[0] ? labelize(entries[0][0]) : "Item");
        const detail =
          value.detail ??
          value.value ??
          value.summary ??
          entries
            .filter(([key]) => !["label", "name", "title", "detail", "value", "summary"].includes(key))
            .slice(0, 3)
            .map(([key, entryValue]) => {
              return `${labelize(key)}: ${formatBoundValue(entryValue, data)}`;
            })
            .join(" · ");
        return `<li><span>${escapeHtml(formatBoundValue(label, data))}</span>${
          detail ? `<strong>${escapeHtml(formatBoundValue(detail, data))}</strong>` : ""
        }</li>`;
      }
      const value = asObject(row);
      const label = value.label ?? value.name ?? row;
      const detail = value.detail ?? value.value ?? value.path ?? value.key ?? "";
      return `<li><span>${escapeHtml(formatBoundValue(label, data))}</span>${
        detail ? `<strong>${escapeHtml(formatBoundValue(detail, data))}</strong>` : ""
      }</li>`;
    })
    .join("");
}

function renderObjectRows(value, data) {
  return Object.entries(asObject(value))
    .filter(([, entryValue]) => entryValue != null && entryValue !== "")
    .map(([key, entryValue]) => {
      const renderedValue = Array.isArray(entryValue)
        ? entryValue.slice(0, 5).map((item) => formatBoundValue(item, data)).join(", ")
        : entryValue && typeof entryValue === "object"
          ? Object.entries(entryValue)
              .slice(0, 4)
              .map(([innerKey, innerValue]) => `${labelize(innerKey)}: ${formatBoundValue(innerValue, data)}`)
              .join(" · ")
          : formatBoundValue(entryValue, data);
      return { label: labelize(key), value: renderedValue };
    });
}

function renderSection(section, data, state = {}, namespace = "section") {
  const item = asObject(section);
  const title = item.title || item.label || item.name;
  const body = item.body || item.summary || item.description || item.content;
  const rows = asArray(item.items || item.rows);
  const metrics = asArray(item.metrics);
  const checklist = asArray(item.checklist || item.checks || item.tasks);
  const objectRows = rows.length ? rows : renderObjectRows(item.fields || item.values, data);
  if (!title && !body && objectRows.length === 0 && metrics.length === 0 && checklist.length === 0) return "";
  return `<section class="app-shell-section">
    ${title ? `<h3>${escapeHtml(formatBoundValue(title, data))}</h3>` : ""}
    ${body ? `<p>${escapeHtml(formatBoundValue(body, data))}</p>` : ""}
    ${metrics.length ? `<div class="app-shell-section-metrics">${metrics.map((metric) => renderMetric(metric, data)).join("")}</div>` : ""}
    ${objectRows.length ? `<ul>${renderRows(objectRows, data)}</ul>` : ""}
    ${checklist.length ? renderChecklist(checklist, data, state, namespace) : ""}
  </section>`;
}

function renderDataField(key, value, data, state) {
  if (value == null || value === "") return "";
  if (Array.isArray(value)) {
    if (!value.length) return "";
    return renderSection({ title: labelize(key), rows: value.slice(0, 12) }, data, state, key);
  }
  if (typeof value === "object") {
    const rows = renderObjectRows(value, data);
    return rows.length ? renderSection({ title: labelize(key), rows }, data, state, key) : "";
  }
  return renderSection({ title: labelize(key), body: value }, data, state, key);
}

function renderLooseData(spec, data, state) {
  const reserved = new Set([
    "id",
    "module",
    "title",
    "subtitle",
    "summary",
    "description",
    "eyebrow",
    "status",
    "metrics",
    "sections",
    "items",
    "rows",
    "actions",
    "source",
    "sourceLabel",
    "fetch",
    "data",
    "dataBindings",
    "defaults",
    "visual",
    "accent",
    "background",
    "refresh",
    "refreshSeconds",
    "refreshMinutes",
    "fetchedStatus",
    "content",
    "body",
    "tabs",
    "views",
    "activeView",
    "defaultView",
    "initialView",
  ]);
  return Object.entries(spec)
    .filter(([key]) => !reserved.has(key))
    .map(([key, value]) => renderDataField(key, value, data, state))
    .filter(Boolean);
}

function renderAction(action) {
  const item = asObject(action);
  const label = text(item.label, "Open");
  const href = text(item.href || item.url);
  const trigger = text(item.trigger || item.action);
  if (href) {
    return `<a class="app-shell-action" href="${escapeHtml(href)}" target="_blank" rel="noreferrer">${escapeHtml(label)}</a>`;
  }
  if (trigger) {
    return `<button type="button" class="app-shell-action" data-app-action="${escapeHtml(trigger)}">${escapeHtml(label)}</button>`;
  }
  return "";
}

function renderSource(spec) {
  const source = asObject(spec.source);
  const label = text(spec.sourceLabel || source.label || source.kind);
  const url = text(source.url);
  if (!label && !url) return "";
  const rendered = label || url;
  return `<div class="app-shell-source">${escapeHtml(rendered)}</div>`;
}

function appData(spec, fetched) {
  const base = asObject(spec.data || spec.dataBindings || spec.defaults);
  if (!fetched) return base;
  if (fetched && typeof fetched === "object" && !Array.isArray(fetched)) {
    return { ...base, ...fetched };
  }
  return { ...base, value: fetched };
}

function fetchSpecFrom(spec) {
  const direct = asObject(spec.fetch);
  if (direct.url) return direct;
  const source = asObject(spec.source);
  if (source.url) {
    return {
      url: source.url,
      format: source.format,
      refreshSeconds: spec.refreshSeconds ?? spec.refresh,
      refreshMinutes: spec.refreshMinutes,
    };
  }
  return {};
}

function renderApp(el, spec, fetched, state) {
  const baseSpec = spec;
  const views = appViews(baseSpec);
  const activeView = selectedView(baseSpec, state);
  spec = activeView ? viewSpec(baseSpec, activeView) : baseSpec;
  const data = appData(spec, fetched);
  const visual = asObject(spec.visual);
  const metrics = asArray(spec.metrics);
  const sections = asArray(spec.sections);
  const actions = asArray(spec.actions).map(renderAction).filter(Boolean);
  const accent = text(spec.accent || visual.accent, "#58e0ff");
  const background = text(spec.background || visual.background);
  const title = text(spec.title, text(state.title, "Orbit App"));
  const subtitle = spec.subtitle || spec.summary || "";
  const status = state.error || state.status || spec.status || "";
  const renderedSections = sections
    .map((section, index) => renderSection(section, data, state, `${state.activeView || "root"}:section-${index}`))
    .filter(Boolean);
  const topRows = asArray(spec.items || spec.rows);
  const renderedTopRows = topRows.length
    ? renderSection({ title: spec.itemsTitle || spec.rowsTitle || "Details", rows: topRows }, data, state, `${state.activeView || "root"}:rows`)
    : "";
  const renderedContent = renderContentBlock(spec.content || spec.body || spec.description, data);
  const looseSections = renderLooseData(spec, data, state);
  const renderedFallbackData =
    !renderedContent && !metrics.length && !renderedSections.length && !renderedTopRows
      ? renderDataField(spec.dataTitle || "Data", spec.data || spec.defaults || spec.dataBindings, data, state)
      : "";
  const hasBody =
    subtitle ||
    renderedContent ||
    metrics.length > 0 ||
    renderedSections.length > 0 ||
    renderedTopRows ||
    renderedFallbackData ||
    looseSections.length > 0 ||
    actions.length > 0;

  el.innerHTML = `
    <style>
      .app-shell {
        box-sizing: border-box;
        width: 100%;
        height: 100%;
        min-height: 180px;
        overflow: auto;
        padding: 18px;
        border-radius: 8px;
        color: #f8fbff;
        background:
          radial-gradient(circle at 12% 0%, color-mix(in srgb, ${accent} 26%, transparent), transparent 42%),
          ${background || "linear-gradient(145deg, rgba(13, 18, 24, 0.98), rgba(21, 31, 37, 0.98))"};
        border: 1px solid rgba(190, 221, 244, 0.2);
        box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.08);
        font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        scrollbar-width: thin;
      }
      .app-shell-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
      }
      .app-shell-kicker,
      .app-shell-source {
        color: rgba(216, 232, 246, 0.66);
        font-size: 11px;
        letter-spacing: 0;
        text-transform: uppercase;
      }
      .app-shell h2 {
        margin: 4px 0 0;
        font-size: 22px;
        line-height: 1.1;
        font-weight: 760;
        letter-spacing: 0;
      }
      .app-shell-status {
        color: ${state.error ? "#ffd7c2" : "rgba(224, 240, 252, 0.72)"};
        font-size: 12px;
        text-align: right;
      }
      .app-shell-summary,
      .app-shell-empty,
      .app-shell-content p,
      .app-shell-content li {
        margin: 14px 0 0;
        color: rgba(230, 242, 250, 0.78);
        font-size: 13px;
        line-height: 1.5;
      }
      .app-shell-tabs {
        display: flex;
        gap: 7px;
        margin-top: 14px;
        overflow-x: auto;
        scrollbar-width: thin;
      }
      .app-shell-tab {
        flex: 0 0 auto;
        min-height: 32px;
        padding: 7px 10px;
        border-radius: 8px;
        border: 1px solid rgba(255, 255, 255, 0.12);
        color: rgba(235, 244, 252, 0.78);
        background: rgba(255, 255, 255, 0.06);
        font-size: 12px;
        font-weight: 700;
        cursor: pointer;
      }
      .app-shell-tab.is-active {
        color: #071019;
        background: ${accent};
        border-color: color-mix(in srgb, ${accent} 70%, white);
      }
      .app-shell-content {
        margin-top: 14px;
      }
      .app-shell-content h3 {
        margin: 14px 0 6px;
        color: rgba(244, 250, 255, 0.9);
        font-size: 13px;
        line-height: 1.25;
      }
      .app-shell-content ul {
        margin: 8px 0 0;
        padding-left: 18px;
      }
      .app-shell-metrics,
      .app-shell-section-metrics {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(104px, 1fr));
        gap: 9px;
        margin-top: 14px;
      }
      .app-shell-metric {
        min-width: 0;
        padding: 10px;
        border-radius: 8px;
        background: linear-gradient(180deg, rgba(255, 255, 255, 0.1), rgba(255, 255, 255, 0.055));
        border: 1px solid rgba(255, 255, 255, 0.09);
      }
      .app-shell-metric span,
      .app-shell-section h3 {
        color: rgba(216, 234, 248, 0.68);
        font-size: 11px;
        font-weight: 620;
        text-transform: uppercase;
        letter-spacing: 0;
      }
      .app-shell-metric strong {
        display: block;
        margin-top: 5px;
        overflow-wrap: anywhere;
        font-size: 21px;
        line-height: 1.08;
      }
      .app-shell-metric em {
        display: block;
        margin-top: 5px;
        color: rgba(226, 238, 248, 0.64);
        font-size: 11px;
        font-style: normal;
      }
      .app-shell-section {
        margin-top: 15px;
        padding-top: 13px;
        border-top: 1px solid rgba(255, 255, 255, 0.1);
      }
      .app-shell-section h3 {
        margin: 0 0 8px;
      }
      .app-shell-section p,
      .app-shell-section li {
        color: rgba(232, 242, 250, 0.8);
        font-size: 13px;
        line-height: 1.45;
      }
      .app-shell-section ul {
        display: grid;
        gap: 6px;
        margin: 10px 0 0;
        padding: 0;
        list-style: none;
      }
      .app-shell-section li {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        align-items: baseline;
      }
      .app-shell-section li strong {
        color: #ffffff;
        text-align: right;
        overflow-wrap: anywhere;
      }
      .app-shell-checklist {
        display: grid;
        gap: 7px;
        margin: 10px 0 0;
        padding: 0;
        list-style: none;
      }
      .app-shell-check {
        width: 100%;
        display: flex;
        align-items: center;
        gap: 10px;
        min-height: 42px;
        padding: 9px;
        border-radius: 8px;
        border: 1px solid rgba(255, 255, 255, 0.1);
        color: inherit;
        background: rgba(255, 255, 255, 0.055);
        text-align: left;
        cursor: pointer;
      }
      .app-shell-check.is-checked {
        background: color-mix(in srgb, ${accent} 18%, rgba(255, 255, 255, 0.06));
      }
      .app-shell-check-box {
        width: 20px;
        height: 20px;
        border-radius: 6px;
        border: 1px solid rgba(255, 255, 255, 0.22);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        color: #071019;
        background: ${accent};
        font-size: 12px;
        font-weight: 900;
        flex: 0 0 auto;
      }
      .app-shell-check-copy {
        display: grid;
        gap: 2px;
        min-width: 0;
      }
      .app-shell-check-copy strong {
        color: rgba(224, 239, 250, 0.65);
        font-size: 11px;
        font-weight: 650;
      }
      .app-shell-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
        margin-top: 16px;
      }
      .app-shell-action {
        color: #061019;
        background: ${accent};
        border: 0;
        border-radius: 8px;
        min-height: 36px;
        padding: 8px 12px;
        font-size: 12px;
        font-weight: 720;
        text-decoration: none;
        cursor: pointer;
      }
      .app-shell-action:hover {
        filter: brightness(1.05);
      }
      .app-shell-action:focus-visible {
        outline: 2px solid rgba(255, 255, 255, 0.85);
        outline-offset: 2px;
      }
      .app-shell-source {
        margin-top: 14px;
        text-transform: none;
      }
    </style>
    <article class="app-shell" aria-label="${escapeHtml(title)}">
      <div class="app-shell-head">
        <div>
          <div class="app-shell-kicker">${escapeHtml(text(spec.eyebrow, "Orbit App"))}</div>
          <h2>${escapeHtml(title)}</h2>
        </div>
        ${status ? `<div class="app-shell-status">${escapeHtml(formatBoundValue(status, data))}</div>` : ""}
      </div>
      ${subtitle ? `<p class="app-shell-summary">${escapeHtml(formatBoundValue(subtitle, data))}</p>` : ""}
      ${renderViewTabs(views, state)}
      ${renderedContent}
      ${metrics.length ? `<div class="app-shell-metrics">${metrics.map((metric) => renderMetric(metric, data)).join("")}</div>` : ""}
      ${renderedSections.join("")}
      ${renderedTopRows}
      ${renderedFallbackData}
      ${looseSections.join("")}
      ${hasBody ? "" : `<p class="app-shell-empty">This widget needs app-specific fields before it can render.</p>`}
      ${actions.length ? `<div class="app-shell-actions">${actions.join("")}</div>` : ""}
      ${renderSource(spec)}
    </article>
  `;
}

async function fetchData(ctx, fetchSpec) {
  const spec = asObject(fetchSpec);
  const url = text(spec.url);
  if (!url) return null;
  if (!ctx || typeof ctx.fetchJson !== "function") {
    throw new Error("Public fetch helper is unavailable.");
  }
  if (spec.format === "text") return ctx.fetchText(url);
  return ctx.fetchJson(url);
}

function refreshSeconds(spec, fetchSpec) {
  const seconds = number(fetchSpec.refreshSeconds ?? spec.refreshSeconds ?? spec.refresh);
  if (seconds != null) return Math.max(10, seconds);
  const minutes = number(fetchSpec.refreshMinutes ?? spec.refreshMinutes);
  return minutes != null ? Math.max(10, minutes * 60) : 0;
}

export function render(el, ctx = {}) {
  const widget = asObject(ctx.widget);
  const spec = asObject(widget.spec);
  const fetchSpec = fetchSpecFrom(spec);
  const state = { title: widget.title || widget.id, status: "" };
  let disposed = false;
  let timer = null;
  let currentData = null;
  let update = async () => {};

  const draw = (data = currentData) => {
    currentData = data;
    renderApp(el, spec, data, state);
    el.querySelectorAll("[data-app-action='refresh']").forEach((button) => {
      button.addEventListener("click", () => void update(), { once: true });
    });
    el.querySelectorAll("[data-app-view]").forEach((button) => {
      button.addEventListener("click", () => {
        state.activeView = button.getAttribute("data-app-view") || "";
        draw(currentData);
      });
    });
    el.querySelectorAll("[data-app-check]").forEach((button) => {
      button.addEventListener("click", () => {
        const key = button.getAttribute("data-app-check") || "";
        if (!key) return;
        if (!state.checked || typeof state.checked !== "object") state.checked = {};
        state.checked[key] = !state.checked[key];
        draw(currentData);
      });
    });
  };

  update = async () => {
    try {
      state.error = "";
      state.status = fetchSpec.url ? "Updating" : "";
      draw();
      const data = await fetchData(ctx, fetchSpec);
      if (disposed) return;
      state.status = fetchSpec.url ? text(spec.fetchedStatus, "Live") : "";
      draw(data);
    } catch (error) {
      if (disposed) return;
      state.status = "";
      state.error = error instanceof Error ? error.message : String(error);
      draw(currentData);
    }
  };

  void update();
  const cadence = refreshSeconds(spec, fetchSpec);
  if (cadence > 0) {
    timer = window.setInterval(update, cadence * 1000);
  }

  return () => {
    disposed = true;
    if (timer) window.clearInterval(timer);
  };
}
