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
  return label
    .split(/\s+/)
    .map((word) => (word ? word.charAt(0).toUpperCase() + word.slice(1) : ""))
    .join(" ");
}

function comparableLabel(value) {
  return text(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

function firstText(...values) {
  for (const value of values) {
    const resolved = text(value);
    if (resolved) return resolved;
  }
  return "";
}

const DESIGN_TYPES = new Set([
  "hero-card",
  "glass-card",
  "dashboard-grid",
  "profile-panel",
  "checklist-board",
  "feed-panel",
]);

function normalizeDesignType(value) {
  const normalized = text(value)
    .toLowerCase()
    .replace(/[_\s]+/g, "-");
  return DESIGN_TYPES.has(normalized) ? normalized : "";
}

function appShellDesignType(spec, visual, shape) {
  const explicit = normalizeDesignType(
    spec.design_type ||
      spec.designType ||
      spec.design ||
      spec.style ||
      visual.design_type ||
      visual.designType ||
      visual.type ||
      visual.style ||
      visual.layout,
  );
  if (explicit) return explicit;
  if (asArray(spec.checklist || spec.checks || spec.tasks).length) return "checklist-board";
  if (shape.sections.length >= 3 || shape.metrics.length >= 4) return "dashboard-grid";
  if (shape.topRows.length >= 4 || shape.views.length >= 2) return "profile-panel";
  if (shape.metrics.length >= 1) return "hero-card";
  return "glass-card";
}

function appShellIllustration(spec, visual) {
  return firstText(
    spec.illustration,
    spec.icon,
    spec.symbol,
    visual.illustration,
    visual.icon,
    visual.symbol,
  );
}

function hexToRgb(value) {
  const raw = text(value).replace(/^#/, "");
  if (!/^[0-9a-f]{3}([0-9a-f]{3})?$/i.test(raw)) return null;
  const full =
    raw.length === 3
      ? raw
          .split("")
          .map((part) => part + part)
          .join("")
      : raw;
  return {
    r: parseInt(full.slice(0, 2), 16),
    g: parseInt(full.slice(2, 4), 16),
    b: parseInt(full.slice(4, 6), 16),
  };
}

function rgbStringToRgb(value) {
  const match = text(value).match(/rgba?\(([^)]+)\)/i);
  if (!match) return null;
  const parts = match[1]
    .split(",")
    .map((part) => Number(part.trim().replace(/%$/, "")))
    .slice(0, 3);
  if (parts.length < 3 || parts.some((part) => !Number.isFinite(part))) return null;
  return {
    r: Math.max(0, Math.min(255, parts[0])),
    g: Math.max(0, Math.min(255, parts[1])),
    b: Math.max(0, Math.min(255, parts[2])),
  };
}

function relativeLuminance({ r, g, b }) {
  const channel = (value) => {
    const normalized = value / 255;
    return normalized <= 0.03928
      ? normalized / 12.92
      : Math.pow((normalized + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function backgroundLuminance(background) {
  const raw = text(background).toLowerCase();
  if (!raw) return null;
  const colorValues = [
    ...(raw.match(/#[0-9a-f]{3}(?:[0-9a-f]{3})?\b/gi) || []),
    ...(raw.match(/rgba?\([^)]+\)/gi) || []),
  ];
  if (!colorValues.length) return null;
  const parsed = colorValues
    .map((value) => hexToRgb(value) || rgbStringToRgb(value))
    .filter(Boolean);
  if (!parsed.length) return null;
  return parsed.reduce((sum, rgb) => sum + relativeLuminance(rgb), 0) / parsed.length;
}

function backgroundLooksLight(background) {
  const luminance = backgroundLuminance(background);
  return luminance !== null && luminance > 0.58;
}

function appShellTone(spec, visual) {
  const explicit = firstText(
    spec.tone,
    spec.theme,
    spec.colorScheme,
    visual.tone,
    visual.theme,
    visual.colorScheme,
  ).toLowerCase();
  if (explicit === "light" || explicit === "dark") return explicit;
  return "dark";
}

function defaultBackgroundForDesign(designType, tone, accent) {
  if (tone === "light") {
    if (designType === "hero-card") {
      return `linear-gradient(135deg, color-mix(in srgb, ${accent} 18%, #f7fbff), #edf4ff 44%, #f8fff3)`;
    }
    if (designType === "dashboard-grid") {
      return "linear-gradient(145deg, #f8fafc, #e9f1fb)";
    }
    return "linear-gradient(145deg, rgba(255,255,255,0.92), rgba(238,247,255,0.9))";
  }
  if (designType === "hero-card") {
    return `linear-gradient(135deg, color-mix(in srgb, ${accent} 18%, #101923), #091015 58%, #141b24)`;
  }
  if (designType === "dashboard-grid") {
    return "linear-gradient(145deg, rgba(10, 14, 20, 0.98), rgba(18, 28, 36, 0.98))";
  }
  return "linear-gradient(145deg, rgba(13, 18, 24, 0.98), rgba(21, 31, 37, 0.98))";
}

function appShellBackground(designType, tone, accent, requestedBackground) {
  const raw = text(requestedBackground);
  if (!raw) return defaultBackgroundForDesign(designType, tone, accent);
  if (/gradient\(|url\(|color-mix\(|var\(/i.test(raw)) {
    if (tone === "light") return raw;
    const luminance = backgroundLuminance(raw);
    const startAlpha = luminance === null ? 0.66 : luminance > 0.58 ? 0.78 : 0.38;
    const endAlpha = Math.max(0.3, startAlpha - 0.08);
    return `linear-gradient(135deg, rgba(3, 8, 12, ${startAlpha}), color-mix(in srgb, ${accent} 16%, rgba(6, 16, 20, ${endAlpha})) 54%, rgba(8, 11, 18, ${endAlpha})), ${raw}`;
  }
  if (tone === "light") {
    return `linear-gradient(135deg, color-mix(in srgb, ${raw} 18%, #f8fbff), #eff6ff 48%, color-mix(in srgb, ${accent} 12%, #f7fff8))`;
  }
  return `linear-gradient(135deg, color-mix(in srgb, ${raw} 24%, #061015), color-mix(in srgb, ${accent} 16%, #071014) 52%, color-mix(in srgb, ${raw} 22%, #121820))`;
}

function toneTokens(tone, actionText) {
  if (tone === "light") {
    return {
      fg: "#102033",
      body: "rgba(16, 32, 51, 0.78)",
      muted: "rgba(16, 32, 51, 0.62)",
      soft: "rgba(16, 32, 51, 0.54)",
      panel: "rgba(255, 255, 255, 0.54)",
      panelStrong: "rgba(255, 255, 255, 0.76)",
      line: "rgba(28, 51, 75, 0.16)",
      shadow: "0 22px 70px rgba(22, 43, 76, 0.22)",
      status: "#285275",
      actionText,
    };
  }
  return {
    fg: "#f8fbff",
    body: "rgba(230, 242, 250, 0.8)",
    muted: "rgba(216, 232, 246, 0.68)",
    soft: "rgba(226, 238, 248, 0.64)",
    panel: "rgba(255, 255, 255, 0.07)",
    panelStrong: "rgba(255, 255, 255, 0.1)",
    line: "rgba(255, 255, 255, 0.11)",
    shadow: "0 22px 70px rgba(0, 0, 0, 0.28)",
    status: "rgba(224, 240, 252, 0.72)",
    actionText,
  };
}

function appShellUsesVisualScene(designType) {
  return designType === "hero-card" || designType === "glass-card" || designType === "profile-panel";
}

function renderVisualScene(illustration) {
  const glyph = text(illustration);
  const showGlyph = glyph && Array.from(glyph).length <= 3;
  return `<div class="app-shell-visual-scene" aria-hidden="true">
    <span class="app-shell-scene-orb app-shell-scene-orb-primary"></span>
    <span class="app-shell-scene-orb app-shell-scene-orb-secondary"></span>
    <span class="app-shell-scene-panel app-shell-scene-panel-a"></span>
    <span class="app-shell-scene-panel app-shell-scene-panel-b"></span>
    <span class="app-shell-scene-lines"></span>
    ${showGlyph ? `<span class="app-shell-visual-glyph">${escapeHtml(glyph)}</span>` : ""}
  </div>`;
}

function appViews(spec) {
  return asArray(spec.views || spec.tabs).filter((view) => view && typeof view === "object");
}

function viewKey(view, index) {
  return firstText(view.id, view.key, view.value, view.label, view.title) || `view-${index}`;
}

function appShellTitle(spec, state, views) {
  const explicit = text(spec.title);
  const stateTitle = text(state.title);
  const tabLabels = new Set(
    views
      .map((view, index) => comparableLabel(firstText(view.label, view.title, viewKey(view, index))))
      .filter(Boolean),
  );
  const fallbackTitle =
    [stateTitle, text(state.id)]
      .map((value) => text(value))
      .find((value) => value && !tabLabels.has(comparableLabel(value))) || stateTitle || text(state.id);
  if (!explicit) return fallbackTitle ? labelize(fallbackTitle) : "App";
  if (tabLabels.has(comparableLabel(explicit))) {
    return fallbackTitle ? labelize(fallbackTitle) : explicit;
  }
  return explicit;
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
    title: firstText(base.title, base.name, base.label, view.title, view.label),
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

function renderPrimaryMetric(metric, data) {
  const item = asObject(metric);
  if (!Object.keys(item).length) return "";
  const label = text(item.label || item.name, "Current");
  const detail = item.detail ?? item.description ?? "";
  return `<div class="app-shell-primary-metric">
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

function renderInfoStrip(rows, data) {
  const items = asArray(rows).slice(0, 4);
  if (!items.length) return "";
  return `<div class="app-shell-info-strip">${items
    .map((row) => {
      const item = asObject(row);
      const entries = Object.entries(item).filter(([, entryValue]) => {
        return entryValue != null && entryValue !== "";
      });
      const label =
        item.label ??
        item.name ??
        item.title ??
        (entries[0] ? labelize(entries[0][0]) : "Detail");
      const value =
        item.value ??
        item.detail ??
        item.summary ??
        entries
          .filter(([key]) => !["label", "name", "title", "detail", "value", "summary"].includes(key))
          .map(([, entryValue]) => entryValue)[0] ??
        "";
      return `<div class="app-shell-info-item">
        <span>${escapeHtml(formatBoundValue(label, data))}</span>
        ${value ? `<strong>${escapeHtml(formatBoundValue(value, data))}</strong>` : ""}
      </div>`;
    })
    .join("")}</div>`;
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
    "label",
    "name",
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
    "design_type",
    "designType",
    "design",
    "style",
    "icon",
    "illustration",
    "symbol",
    "theme",
    "tone",
    "colorScheme",
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
  const requestedBackground = text(spec.background || visual.background);
  const topRows = asArray(spec.items || spec.rows);
  const shape = { views, metrics, sections, topRows };
  const designType = appShellDesignType(spec, visual, shape);
  const tone = appShellTone(spec, visual);
  const actionText = backgroundLooksLight(accent) ? "#071019" : "#f8fbff";
  const tokens = toneTokens(tone, actionText);
  const background = appShellBackground(designType, tone, accent, requestedBackground);
  const illustration = appShellIllustration(spec, visual);
  const title = appShellTitle(spec, state, views);
  const subtitle = spec.subtitle || spec.summary || "";
  const status = state.error || state.status || spec.status || "";
  const kicker = text(spec.eyebrow || visual.eyebrow || spec.category || visual.category);
  const renderedVisualScene = appShellUsesVisualScene(designType) ? renderVisualScene(illustration) : "";
  const renderedSections = sections
    .map((section, index) => renderSection(section, data, state, `${state.activeView || "root"}:section-${index}`))
    .filter(Boolean);
  const usesCompactInfoStrip = ["profile-panel", "glass-card", "hero-card"].includes(designType) && topRows.length > 0;
  const renderedInfoStrip = usesCompactInfoStrip ? renderInfoStrip(topRows, data) : "";
  const renderedTopRows = topRows.length && !usesCompactInfoStrip
    ? renderSection({ title: spec.itemsTitle || spec.rowsTitle || "Details", rows: topRows }, data, state, `${state.activeView || "root"}:rows`)
    : "";
  const renderedContent = renderContentBlock(spec.content || spec.body || spec.description, data);
  const looseSections = renderLooseData(spec, data, state);
  const usesPrimaryMetric = ["hero-card", "glass-card"].includes(designType) && metrics.length > 0;
  const renderedPrimaryMetric = usesPrimaryMetric ? renderPrimaryMetric(metrics[0], data) : "";
  const metricCards = usesPrimaryMetric ? metrics.slice(1) : metrics;
  const renderedFallbackData =
    !renderedContent && !metrics.length && !renderedSections.length && !renderedTopRows
      ? renderDataField(spec.dataTitle || "Data", spec.data || spec.defaults || spec.dataBindings, data, state)
      : "";
  const hasBody =
    subtitle ||
    renderedContent ||
    metrics.length > 0 ||
    renderedSections.length > 0 ||
    renderedInfoStrip ||
    renderedTopRows ||
    renderedFallbackData ||
    looseSections.length > 0 ||
    actions.length > 0;

  el.innerHTML = `
    <style>
      .app-shell {
        --app-accent: ${accent};
        --app-fg: ${tokens.fg};
        --app-body: ${tokens.body};
        --app-muted: ${tokens.muted};
        --app-soft: ${tokens.soft};
        --app-panel: ${tokens.panel};
        --app-panel-strong: ${tokens.panelStrong};
        --app-line: ${tokens.line};
        --app-status: ${state.error ? "#8a2c16" : tokens.status};
        --app-action-text: ${tokens.actionText};
        box-sizing: border-box;
        width: 100%;
        height: 100%;
        min-height: 180px;
        overflow: auto;
        padding: 28px;
        border-radius: 8px;
        color: var(--app-fg);
        background:
          radial-gradient(circle at 16% -14%, color-mix(in srgb, var(--app-accent) 28%, transparent), transparent 36%),
          radial-gradient(circle at 92% 12%, rgba(255, 255, 255, 0.13), transparent 24%),
          ${background};
        border: 1px solid var(--app-line);
        box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.13), ${tokens.shadow};
        font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        scrollbar-width: thin;
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 17px;
      }
      .app-shell-light {
        text-shadow: none;
      }
      .app-shell-dark {
        text-shadow: 0 1px 2px rgba(0, 0, 0, 0.28);
      }
      .app-shell-hero-card,
      .app-shell-glass-card {
        padding: 34px;
      }
      .app-shell-stage {
        min-width: 0;
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(150px, 0.72fr) minmax(118px, 0.48fr);
        align-items: center;
        gap: 24px;
      }
      .app-shell:not(.has-feature) .app-shell-stage {
        grid-template-columns: minmax(0, 1fr) minmax(150px, 0.68fr);
      }
      .app-shell-dashboard-grid .app-shell-stage,
      .app-shell-checklist-board .app-shell-stage,
      .app-shell-feed-panel .app-shell-stage {
        display: block;
      }
      .app-shell-copy,
      .app-shell-body,
      .app-shell-feature {
        min-width: 0;
      }
      .app-shell-body {
        display: grid;
        gap: 14px;
      }
      .app-shell-profile-panel .app-shell-body,
      .app-shell-glass-card .app-shell-body,
      .app-shell-hero-card .app-shell-body {
        gap: 15px;
      }
      .app-shell-dashboard-grid .app-shell-metrics,
      .app-shell-dashboard-grid .app-shell-section-metrics {
        grid-template-columns: repeat(auto-fit, minmax(132px, 1fr));
      }
      .app-shell-profile-panel .app-shell-metrics {
        grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      }
      .app-shell-checklist-board .app-shell-checklist,
      .app-shell-feed-panel .app-shell-section ul {
        gap: 10px;
      }
      .app-shell-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
      }
      .app-shell-visual-scene {
        position: relative;
        min-width: 0;
        width: 170px;
        max-width: 100%;
        aspect-ratio: 1.22;
        justify-self: center;
        border-radius: 26px;
        overflow: hidden;
        background:
          radial-gradient(circle at 25% 72%, color-mix(in srgb, var(--app-accent) 46%, transparent), transparent 28%),
          linear-gradient(145deg, rgba(255, 255, 255, 0.24), rgba(255, 255, 255, 0.06));
        border: 1px solid var(--app-line);
        box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18), 0 18px 46px rgba(0, 0, 0, 0.2);
      }
      .app-shell-scene-orb,
      .app-shell-scene-panel,
      .app-shell-scene-lines,
      .app-shell-visual-glyph {
        position: absolute;
        display: block;
      }
      .app-shell-scene-orb-primary {
        width: 76px;
        height: 76px;
        right: 18px;
        top: 22px;
        border-radius: 999px;
        background: color-mix(in srgb, var(--app-accent) 80%, #ffffff);
        box-shadow: 0 0 34px color-mix(in srgb, var(--app-accent) 36%, transparent);
      }
      .app-shell-scene-orb-secondary {
        width: 88px;
        height: 56px;
        left: 22px;
        bottom: 22px;
        border-radius: 999px 999px 30px 30px;
        background: linear-gradient(180deg, rgba(255,255,255,0.34), rgba(255,255,255,0.13));
        border: 1px solid rgba(255,255,255,0.18);
      }
      .app-shell-scene-panel {
        width: 38px;
        height: 58px;
        border-radius: 10px;
        background: linear-gradient(180deg, rgba(8, 15, 22, 0.58), rgba(255,255,255,0.12));
        border: 1px solid rgba(255,255,255,0.18);
      }
      .app-shell-scene-panel-a {
        left: 38px;
        top: 26px;
        transform: rotate(-4deg);
      }
      .app-shell-scene-panel-b {
        left: 76px;
        top: 42px;
        height: 48px;
        transform: rotate(5deg);
      }
      .app-shell-scene-lines {
        right: 24px;
        bottom: 24px;
        width: 44px;
        height: 28px;
        background:
          linear-gradient(var(--app-fg), var(--app-fg)) 0 0 / 100% 2px no-repeat,
          linear-gradient(var(--app-fg), var(--app-fg)) 0 12px / 72% 2px no-repeat,
          linear-gradient(var(--app-fg), var(--app-fg)) 0 24px / 88% 2px no-repeat;
        opacity: 0.52;
      }
      .app-shell-visual-glyph {
        left: 50%;
        top: 50%;
        width: 56px;
        height: 56px;
        transform: translate(-50%, -50%);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border-radius: 18px;
        background: rgba(255,255,255,0.16);
        border: 1px solid rgba(255,255,255,0.16);
        color: var(--app-fg);
        font-size: 32px;
        line-height: 1;
        backdrop-filter: blur(10px);
      }
      .app-shell-kicker,
      .app-shell-source {
        color: var(--app-muted);
        font-size: 11px;
        letter-spacing: 0;
        text-transform: uppercase;
      }
      .app-shell h2 {
        margin: 0;
        color: var(--app-fg);
        font-size: 28px;
        line-height: 1.02;
        font-weight: 780;
        letter-spacing: 0;
      }
      .app-shell-dashboard-grid h2,
      .app-shell-checklist-board h2,
      .app-shell-feed-panel h2 {
        font-size: 24px;
        line-height: 1.08;
      }
      .app-shell-status {
        color: var(--app-status);
        font-size: 12px;
        text-align: right;
      }
      .app-shell-summary,
      .app-shell-empty,
      .app-shell-content p,
      .app-shell-content li {
        margin: 10px 0 0;
        color: var(--app-body);
        font-size: 13px;
        line-height: 1.5;
      }
      .app-shell-tabs {
        display: flex;
        gap: 7px;
        margin-top: 16px;
        overflow-x: auto;
        scrollbar-width: thin;
      }
      .app-shell-tab {
        flex: 0 0 auto;
        min-height: 32px;
        padding: 7px 10px;
        border-radius: 8px;
        border: 1px solid var(--app-line);
        color: var(--app-muted);
        background: var(--app-panel);
        font-size: 12px;
        font-weight: 700;
        cursor: pointer;
      }
      .app-shell-tab.is-active {
        color: var(--app-action-text);
        background: var(--app-accent);
        border-color: color-mix(in srgb, var(--app-accent) 70%, white);
      }
      .app-shell-content {
        margin-top: 0;
      }
      .app-shell-content h3 {
        margin: 14px 0 6px;
        color: var(--app-fg);
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
        margin-top: 0;
      }
      .app-shell-metric {
        min-width: 0;
        padding: 13px;
        border-radius: 8px;
        background: linear-gradient(180deg, var(--app-panel-strong), var(--app-panel));
        border: 1px solid var(--app-line);
      }
      .app-shell-info-strip {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
        gap: 9px;
        margin-top: 0;
      }
      .app-shell-info-item {
        min-width: 0;
        padding: 12px 13px;
        border-radius: 8px;
        background: color-mix(in srgb, var(--app-panel) 82%, transparent);
        border: 1px solid var(--app-line);
      }
      .app-shell-info-item span {
        display: block;
        color: var(--app-muted);
        font-size: 10.5px;
        font-weight: 650;
        text-transform: uppercase;
        letter-spacing: 0;
      }
      .app-shell-info-item strong {
        display: block;
        margin-top: 5px;
        color: var(--app-fg);
        font-size: 14px;
        line-height: 1.25;
        overflow-wrap: anywhere;
      }
      .app-shell-primary-metric {
        width: 100%;
        max-width: 100%;
        margin: 0;
        padding: 16px;
        border-radius: 16px;
        background: color-mix(in srgb, var(--app-accent) 16%, var(--app-panel-strong));
        border: 1px solid color-mix(in srgb, var(--app-accent) 35%, var(--app-line));
        container-type: inline-size;
      }
      .app-shell-metric span,
      .app-shell-primary-metric span,
      .app-shell-section h3 {
        color: var(--app-muted);
        font-size: 11px;
        font-weight: 620;
        text-transform: uppercase;
        letter-spacing: 0;
      }
      .app-shell-metric strong,
      .app-shell-primary-metric strong {
        display: block;
        margin-top: 5px;
        overflow-wrap: anywhere;
        font-size: 21px;
        line-height: 1.08;
      }
      .app-shell-primary-metric strong {
        font-size: clamp(24px, 18cqw, 36px);
        line-height: 0.98;
        white-space: nowrap;
      }
      .app-shell-metric em,
      .app-shell-primary-metric em {
        display: block;
        margin-top: 5px;
        color: var(--app-soft);
        font-size: 11px;
        font-style: normal;
      }
      .app-shell-section {
        margin-top: 0;
        padding-top: 13px;
        border-top: 1px solid var(--app-line);
      }
      .app-shell-section h3 {
        margin: 0 0 8px;
      }
      .app-shell-section p,
      .app-shell-section li {
        color: var(--app-body);
        font-size: 13px;
        line-height: 1.45;
      }
      .app-shell-profile-panel .app-shell-section p,
      .app-shell-glass-card .app-shell-section p,
      .app-shell-hero-card .app-shell-section p {
        max-width: 68ch;
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
        color: var(--app-fg);
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
        border: 1px solid var(--app-line);
        color: inherit;
        background: var(--app-panel);
        text-align: left;
        cursor: pointer;
      }
      .app-shell-check.is-checked {
        background: color-mix(in srgb, var(--app-accent) 18%, var(--app-panel));
      }
      .app-shell-check-box {
        width: 20px;
        height: 20px;
        border-radius: 6px;
        border: 1px solid var(--app-line);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        color: var(--app-action-text);
        background: var(--app-accent);
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
        color: var(--app-soft);
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
        color: var(--app-action-text);
        background: var(--app-accent);
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
        margin-top: 0;
        text-transform: none;
      }
      @media (max-width: 560px) {
        .app-shell {
          padding: 20px;
          justify-content: flex-start;
        }
        .app-shell-stage {
          grid-template-columns: 1fr;
          gap: 15px;
        }
        .app-shell-visual-scene {
          width: 148px;
          justify-self: start;
        }
        .app-shell h2 {
          font-size: 24px;
        }
      }
    </style>
    <article class="app-shell app-shell-${escapeHtml(designType)} app-shell-${escapeHtml(tone)}${renderedPrimaryMetric ? " has-feature" : ""}" aria-label="${escapeHtml(title)}">
      <div class="app-shell-stage">
        <div class="app-shell-copy">
          ${kicker ? `<div class="app-shell-kicker">${escapeHtml(kicker)}</div>` : ""}
          <h2>${escapeHtml(title)}</h2>
          ${subtitle ? `<p class="app-shell-summary">${escapeHtml(formatBoundValue(subtitle, data))}</p>` : ""}
          ${renderViewTabs(views, state)}
        </div>
        ${renderedVisualScene}
        ${renderedPrimaryMetric ? `<div class="app-shell-feature">${renderedPrimaryMetric}</div>` : ""}
      </div>
      <div class="app-shell-body">
        ${status ? `<div class="app-shell-status">${escapeHtml(formatBoundValue(status, data))}</div>` : ""}
        ${renderedContent}
        ${metricCards.length ? `<div class="app-shell-metrics">${metricCards.map((metric) => renderMetric(metric, data)).join("")}</div>` : ""}
        ${renderedInfoStrip}
        ${renderedSections.join("")}
        ${renderedTopRows}
        ${renderedFallbackData}
        ${looseSections.join("")}
        ${hasBody ? "" : `<p class="app-shell-empty">This widget needs app-specific fields before it can render.</p>`}
        ${actions.length ? `<div class="app-shell-actions">${actions.join("")}</div>` : ""}
        ${renderSource(spec)}
      </div>
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
  const state = { id: widget.id, title: widget.title || widget.id, status: "" };
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
