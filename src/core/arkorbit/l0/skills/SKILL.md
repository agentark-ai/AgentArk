# ArkOrbit L0 Widget Catalog

ArkOrbit widgets run in the browser Orbit canvas. User-authored files live under the orbit's L2 folder and firmware modules live in this L0 catalog.

Write user widgets as browser JavaScript modules under `mod/<name>/index.js`, assets under `assets/`, and JSON state under `data/`. Widget modules must export `render(el, ctx = {})`; the Orbit canvas imports them and mounts them automatically.

When using structured ArkOrbit file operations, every write operation must include the complete file content. Never send a write with only a path.

For edits to existing widgets, use the structured ArkOrbit operation path with a small exact `find`/`replace` edit instead of re-emitting the whole file.

Security policy:

- Treat orbit code as browser-only display code.
- Never write OAuth tokens, API keys, cookies, bearer headers, passwords, session material, private keys, or credential-like secrets into orbit HTML, JavaScript, CSS, JSON, or assets.
- If live authenticated data is needed, retrieve it through authorized server-side AgentArk tools first, then write only safe rendered summaries or non-secret derived data into the orbit.
- Keep external scripts out of generated orbit pages unless the user explicitly asks for that dependency and it is necessary for the artifact.
- Prefer the widget context helpers (`ctx.importMod(path)` and `ctx.resolveText(path)`) over direct control-plane API calls from inside widgets.

Available firmware modules:

- `app-shell/index.js`: renders compact declarative mini-app specs from a widget registry entry's `spec` object. Prefer this for minimal customer-facing widgets, dashboards, trackers, plans, and checklists when the spec can use app-specific summary/content, metrics, sections, rows/items, views/tabs, checklist items, refresh actions, source/fetch bindings, and visual direction to produce a useful first screen. Include `visual.design_type` (`hero-card`, `glass-card`, `dashboard-grid`, `profile-panel`, `checklist-board`, or `feed-panel`) so the renderer can match the requested app type with domain-specific title, labels, palette, and hierarchy instead of showing a generic card. Default to Orbit-native dark/glass surfaces in the same visual family as the surrounding space-agent UI unless the user explicitly asks for light. Keep the initial surface concise and designed; put deeper detail behind tabs or compact sections. Full deployable/backend apps belong in main chat/app_deploy, not Orbit. Use custom JavaScript for compact Orbit widgets that need rendering, public API fetching, parsing, simulation, state, animation, or behavior the declarative shell cannot express.
- `markdown/index.js`: renders default orbit introduction content.
- `iframe-html/index.js`: renders a self-contained HTML fragment.
- `chart/index.js`: exports `barChart(el, values, options)`.
- `table/index.js`: exports `table(el, columns, rows)`.
- `todo/index.js`: renders a small local todo list.
- Public HTTPS data can be fetched from widgets through the render context: `ctx.fetchText(url)`, `ctx.fetchJson(url)`, or `ctx.fetchPublic(url)`. Prefer these helpers over direct browser `fetch()` for news, RSS, pricing, market data, or other public feeds because the Orbit host routes them through AgentArk and avoids common CORS failures. Do not use them for private hosts, authenticated APIs, or secrets.
- For general latest-news widgets, do not default to Reddit, X/Twitter, forum posts, or social-media search unless the user explicitly asks for that source. Prefer public news/RSS/search feeds from news providers or aggregators, label the source in the UI, and show a clear error if a public source is unavailable. Do not use JSONP or script-tag injection for news data.

For ordinary widget requests, do not rewrite `index.html`. Build minimal customer-facing Orbit widgets with an Orbit-native dark/glass visual system, strong typography, concise copy, and a clear focal point. Before writing files, reject designs that read like generic admin cards, reports, fact sheets, pale brochures, oversized text dumps, low-contrast surfaces, or large app implementations. Use a compact `data/widgets.json` app-shell registry spec only when it can produce that result; otherwise write a compact custom browser widget module. Longer code is acceptable when it exists for real public API handling, parsing, state, animation, or interaction inside a small widget. Detailed apps belong in main chat/app_deploy. Use surgical edits for existing modules unless the user explicitly asks for supporting assets or data files.

If the user asks to add back a widget, reuse `mod/<name>/index.js` by restoring its `data/widgets.json` registry entry when that module still exists. If the module was deleted, recreate it from the user's request and conversation context.
