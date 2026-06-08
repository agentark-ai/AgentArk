use axum::{
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use super::{request_matches_active_tunnel, AppState, ErrorResponse};

fn actor_label(
    maybe_caller: Option<&crate::actions::ActionCallerPrincipal>,
    fallback: &str,
) -> String {
    maybe_caller
        .map(|caller| format!("{}:{}", caller.auth_source, caller.user_id))
        .unwrap_or_else(|| fallback.to_string())
}

async fn plane_from_state(state: &AppState) -> crate::core::CompanionControlPlane {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    crate::core::CompanionControlPlane::new(storage)
}

fn json_error(status: axum::http::StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
struct CompanionPhoneSummary {
    paired: usize,
    online: usize,
    capabilities: Vec<String>,
    can_read_phone_messages: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct CompanionMobileAccessChannel {
    id: String,
    label: String,
    kind: String,
    configured: bool,
    ready: bool,
    detail: String,
    settings_path: String,
}

#[derive(Debug, Serialize)]
struct CompanionMobileAccessResponse {
    phone_companion: CompanionPhoneSummary,
    message_channels: Vec<CompanionMobileAccessChannel>,
    sms: CompanionMobileAccessChannel,
    truth_notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct CompanionWsAuth {
    device_id: String,
    token: String,
}

fn companion_tunnel_url_is_https(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .is_some_and(|parsed| parsed.scheme() == "https")
}

async fn companion_request_allowed(
    state: &AppState,
    headers: &HeaderMap,
    addr: SocketAddr,
) -> bool {
    if super::auth::is_trusted_local_ui_request(headers, addr) {
        return true;
    }

    let tunnel = state.tunnel.read().await;
    let Some(tunnel_url) = tunnel.url.as_deref() else {
        return false;
    };
    tunnel.active
        && tunnel.companion_enabled
        && companion_tunnel_url_is_https(tunnel_url)
        && request_matches_active_tunnel(headers, Some(tunnel_url))
}

async fn companion_request_is_active_non_https_tunnel(
    state: &AppState,
    headers: &HeaderMap,
) -> bool {
    let tunnel = state.tunnel.read().await;
    let Some(tunnel_url) = tunnel.url.as_deref() else {
        return false;
    };
    tunnel.active
        && tunnel.companion_enabled
        && !companion_tunnel_url_is_https(tunnel_url)
        && request_matches_active_tunnel(headers, Some(tunnel_url))
}

fn companion_ws_header_auth(headers: &HeaderMap) -> Option<CompanionWsAuth> {
    let device_id = headers
        .get("x-agentark-companion-device")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let (scheme, token) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(CompanionWsAuth {
        device_id,
        token: token.to_string(),
    })
}

pub(super) async fn get_presets() -> Response {
    Json(crate::core::companion_presets_response()).into_response()
}

pub(super) async fn get_protocol() -> Response {
    Json(crate::core::companion_protocol_document()).into_response()
}

const COMPANION_WEB_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>AgentArk Web Companion</title>
  <link rel="icon" type="image/png" href="/favicon.png" />
  <style>
    :root {
      color-scheme: dark;
      --bg: #030504;
      --bg-2: #010201;
      --panel: rgba(10,10,10,.82);
      --panel-2: rgba(14,18,20,.92);
      --field: #06090a;
      --line: rgba(255,255,255,.11);
      --line-strong: rgba(124,231,255,.38);
      --text: #eff7ef;
      --muted: rgba(213,216,223,.72);
      --dim: rgba(155,159,169,.65);
      --cyan: #7ce7ff;
      --violet: #8b5cf6;
      --amber: #f59e0b;
      --ok: #47c47a;
      --warn: #ffbe63;
      --bad: #ff9b9b;
      --shadow: 0 18px 60px rgba(0,0,0,.42);
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100dvh;
      background:
        linear-gradient(180deg, rgba(124,231,255,.07), transparent 30%),
        linear-gradient(135deg, rgba(139,92,246,.10), transparent 34%, rgba(245,158,11,.06)),
        linear-gradient(180deg, var(--bg), var(--bg-2));
      color: var(--text);
      font: 15px/1.5 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      -webkit-font-smoothing: antialiased;
    }
    main {
      width: min(760px, 100%);
      margin: 0 auto;
      padding: max(18px, env(safe-area-inset-top)) 16px max(24px, env(safe-area-inset-bottom));
    }
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      padding: 8px 0 18px;
    }
    .brand {
      display: grid;
      grid-template-columns: 46px minmax(0, 1fr);
      gap: 12px;
      align-items: center;
      min-width: 0;
    }
    .brand-logo {
      width: 46px;
      height: 46px;
      object-fit: contain;
      filter: drop-shadow(0 0 14px rgba(124,231,255,.32));
    }
    .brand-kicker {
      margin: 0 0 1px;
      color: var(--cyan);
      text-transform: uppercase;
      font-size: 10px;
      font-weight: 750;
      letter-spacing: .08em;
    }
    h1 {
      margin: 0;
      font-size: 24px;
      line-height: 1.15;
      font-weight: 720;
      letter-spacing: 0;
    }
    .subtitle {
      margin: 5px 0 0;
      color: var(--muted);
      font-size: 13px;
    }
    .status {
      display: inline-flex;
      align-items: center;
      gap: 7px;
      min-height: 34px;
      padding: 0 11px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: rgba(10,10,10,.72);
      color: var(--muted);
      white-space: nowrap;
      font-size: 13px;
    }
    .dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: var(--warn);
      box-shadow: 0 0 0 3px rgba(255,190,99,.13);
    }
    .status[data-state="connected"] .dot { background: var(--ok); box-shadow: 0 0 0 3px rgba(71,196,122,.15); }
    .status[data-state="error"] .dot { background: var(--bad); box-shadow: 0 0 0 3px rgba(255,155,155,.15); }
    section {
      border: 1px solid var(--line);
      border-radius: 12px;
      background: linear-gradient(135deg, rgba(124,231,255,.07), transparent 42%, rgba(255,255,255,.03)), var(--panel);
      box-shadow: var(--shadow);
      padding: 14px;
      margin: 0 0 12px;
    }
    h2 {
      margin: 0 0 10px;
      font-size: 15px;
      font-weight: 700;
      letter-spacing: 0;
    }
    label {
      display: block;
      color: var(--muted);
      font-size: 12px;
      margin: 10px 0 5px;
    }
    input {
      width: 100%;
      min-height: 44px;
      border: 1px solid var(--line);
      border-radius: 9px;
      background: var(--field);
      color: var(--text);
      padding: 10px 11px;
      font: inherit;
      outline: none;
    }
    input:focus {
      border-color: var(--line-strong);
      box-shadow: 0 0 0 3px rgba(124,231,255,.13);
    }
    .grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 10px;
    }
    .actions {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-top: 12px;
    }
    button {
      min-height: 44px;
      border: 1px solid var(--line);
      border-radius: 9px;
      background: var(--panel-2);
      color: var(--text);
      padding: 0 13px;
      font: inherit;
      font-weight: 650;
      touch-action: manipulation;
    }
    button.primary {
      border-color: rgba(124,231,255,.58);
      background: linear-gradient(180deg, rgba(124,231,255,.20), rgba(124,231,255,.09));
      box-shadow: inset 0 0 22px rgba(124,231,255,.08);
    }
    button.danger {
      border-color: rgba(255,155,155,.42);
      color: #ffd2d2;
    }
    button:disabled {
      opacity: .52;
    }
    .note {
      margin: 9px 0 0;
      color: var(--muted);
      font-size: 13px;
    }
    .device {
      display: grid;
      gap: 6px;
      padding: 10px;
      border: 1px solid var(--line);
      border-radius: 10px;
      background: rgba(0,0,0,.18);
      overflow-wrap: anywhere;
    }
    .device strong {
      font-size: 13px;
    }
    .device span {
      color: var(--muted);
      font-size: 12px;
    }
    .command {
      padding: 11px;
      border: 1px solid rgba(124,231,255,.30);
      border-radius: 10px;
      background: rgba(124,231,255,.08);
      margin-top: 8px;
    }
    .command-title {
      font-weight: 700;
      overflow-wrap: anywhere;
    }
    .command-meta {
      color: var(--muted);
      font-size: 12px;
      margin-top: 3px;
      overflow-wrap: anywhere;
    }
    pre {
      max-height: 250px;
      overflow: auto;
      margin: 0;
      padding: 10px;
      border-radius: 9px;
      background: var(--field);
      border: 1px solid var(--line);
      color: #d8f5ff;
      font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    @media (max-width: 560px) {
      main { padding-left: 18px; padding-right: 18px; }
      header { align-items: flex-start; flex-direction: column; }
      .brand { grid-template-columns: 40px minmax(0, 1fr); gap: 10px; }
      .brand-logo { width: 40px; height: 40px; }
      h1 { font-size: 22px; }
      .grid { grid-template-columns: 1fr; }
      button { width: 100%; }
      .actions { display: grid; grid-template-columns: 1fr; }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <div class="brand">
        <img class="brand-logo" src="/logo.svg" alt="AgentArk" />
        <div>
          <p class="brand-kicker">AgentArk</p>
          <h1>Web Companion</h1>
          <p class="subtitle">No Xcode install. Keep this page open while testing commands.</p>
        </div>
      </div>
      <div id="status" class="status" data-state="idle"><span class="dot"></span><span id="statusText">Not connected</span></div>
    </header>

    <section>
      <h2>Connection</h2>
      <label for="wsUrl">WebSocket URL</label>
      <input id="wsUrl" autocomplete="off" spellcheck="false" />
      <div class="grid">
        <div>
          <label for="sessionId">Pairing session id</label>
          <input id="sessionId" autocomplete="off" spellcheck="false" />
        </div>
        <div>
          <label for="pairingCode">Pairing code</label>
          <input id="pairingCode" autocomplete="off" spellcheck="false" />
        </div>
      </div>
      <div class="actions">
        <button id="claimButton" class="primary">Claim pairing</button>
        <button id="connectButton">Connect saved device</button>
        <button id="notifyButton">Allow notifications</button>
        <button id="clearButton" class="danger">Forget device</button>
      </div>
      <p class="note">After you tap Claim pairing, approve the claimed device in AgentArk. This page retries finalization until approval completes.</p>
    </section>

    <section>
      <h2>Saved Device</h2>
      <div id="deviceBox" class="device">
        <strong>No saved device</strong>
        <span>Pair once to store a scoped browser companion token on this phone.</span>
      </div>
    </section>

    <section>
      <h2>Commands</h2>
      <div id="commands"></div>
      <p id="emptyCommands" class="note">No commands received yet.</p>
    </section>

    <section>
      <h2>Event Log</h2>
      <pre id="log"></pre>
    </section>
  </main>

  <script>
    const storageKey = "agentark.webCompanion.v1";
    const publicKeyKey = "agentark.webCompanion.publicKey.v1";
    let socket = null;
    let retryTimer = null;
    let saved = loadSaved();

    const wsUrlInput = document.getElementById("wsUrl");
    const sessionInput = document.getElementById("sessionId");
    const codeInput = document.getElementById("pairingCode");
    const statusEl = document.getElementById("status");
    const statusText = document.getElementById("statusText");
    const logEl = document.getElementById("log");
    const deviceBox = document.getElementById("deviceBox");
    const commandsEl = document.getElementById("commands");
    const emptyCommands = document.getElementById("emptyCommands");

    function params() {
      return new URLSearchParams(window.location.search);
    }

    function defaultWsUrl() {
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      return `${protocol}//${window.location.host}/companion/ws`;
    }

    function randomId(prefix) {
      const bytes = new Uint8Array(18);
      crypto.getRandomValues(bytes);
      const encoded = Array.from(bytes).map((value) => value.toString(16).padStart(2, "0")).join("");
      return `${prefix}${encoded}`;
    }

    function devicePublicKey() {
      let value = localStorage.getItem(publicKeyKey);
      if (!value) {
        value = randomId("webpk_");
        localStorage.setItem(publicKeyKey, value);
      }
      return value;
    }

    function loadSaved() {
      try {
        return JSON.parse(localStorage.getItem(storageKey) || "null");
      } catch {
        return null;
      }
    }

    function saveDevice(device, token) {
      saved = {
        device_id: device.id,
        display_name: device.display_name || "Web Companion",
        token,
        saved_at: new Date().toISOString()
      };
      localStorage.setItem(storageKey, JSON.stringify(saved));
      renderSaved();
    }

    function setStatus(state, text) {
      statusEl.dataset.state = state;
      statusText.textContent = text;
    }

    function log(message, payload) {
      const line = `[${new Date().toLocaleTimeString()}] ${message}`;
      const detail = payload ? `\n${JSON.stringify(payload, null, 2)}` : "";
      logEl.textContent = `${line}${detail}\n\n${logEl.textContent}`.slice(0, 9000);
    }

    function renderSaved() {
      if (!saved || !saved.device_id || !saved.token) {
        deviceBox.innerHTML = "<strong>No saved device</strong><span>Pair once to store a scoped browser companion token on this phone.</span>";
        return;
      }
      deviceBox.innerHTML = `<strong>${escapeHtml(saved.display_name || "Web Companion")}</strong><span>${escapeHtml(saved.device_id)}</span><span>Saved ${escapeHtml(new Date(saved.saved_at || Date.now()).toLocaleString())}</span>`;
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch]));
    }

    function send(payload) {
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        log("Socket is not open");
        return false;
      }
      socket.send(JSON.stringify(payload));
      return true;
    }

    function openSocket(onOpen) {
      if (socket) {
        socket.close();
      }
      setStatus("idle", "Connecting");
      socket = new WebSocket(wsUrlInput.value.trim() || defaultWsUrl());
      socket.onopen = () => {
        setStatus("connected", "Connected");
        log("WebSocket connected");
        onOpen && onOpen();
      };
      socket.onclose = () => {
        setStatus("idle", "Disconnected");
        log("WebSocket closed");
      };
      socket.onerror = () => {
        setStatus("error", "Connection error");
        log("WebSocket error");
      };
      socket.onmessage = (event) => {
        let message;
        try {
          message = JSON.parse(event.data);
        } catch {
          log("Invalid JSON from server", event.data);
          return;
        }
        handleMessage(message);
      };
    }

    function pairingPayload() {
      return {
        type: "pairing_claim",
        session_id: sessionInput.value.trim(),
        code: codeInput.value.trim(),
        device_public_key: devicePublicKey(),
        metadata: {
          client: "agentark-web-companion",
          model: navigator.userAgent.slice(0, 140),
          page_origin: window.location.origin
        }
      };
    }

    function claimPairing() {
      if (!sessionInput.value.trim() || !codeInput.value.trim()) {
        setStatus("error", "Missing pairing details");
        log("Enter the session id and pairing code first");
        return;
      }
      clearInterval(retryTimer);
      openSocket(() => {
        send(pairingPayload());
        retryTimer = setInterval(() => send(pairingPayload()), 3500);
      });
    }

    function connectSaved() {
      if (!saved || !saved.device_id || !saved.token) {
        setStatus("error", "No saved device");
        log("No saved device token on this phone");
        return;
      }
      openSocket(() => {
        send({
          type: "browser_auth",
          device_id: saved.device_id,
          token: saved.token
        });
      });
    }

    function sendPulse() {
      if (!saved || !saved.device_id) return;
      send({
        type: "pulse",
        device_id: saved.device_id,
        state: "online",
        capabilities: ["approval_prompt", "notifications"],
        commands: [
          {
            id: "approval.prompt",
            label: "Approval prompt",
            capability: "approval_prompt",
            action: "approval.prompt",
            description: "Ask this browser companion for an approval decision.",
            risk: "low"
          },
          {
            id: "notifications.show",
            label: "Show notification",
            capability: "notifications",
            action: "notifications.show",
            description: "Show a browser notification on this companion.",
            risk: "low"
          }
        ],
        metadata: {
          client: "agentark-web-companion",
          notification_permission: "Notification" in window ? Notification.permission : "unavailable"
        }
      });
    }

    function handleMessage(message) {
      if (message.type !== "hello" && message.type !== "pulse_ok") {
        log(`Received ${message.type || "message"}`, message);
      }
      if (message.type === "pairing_claim_result") {
        const result = message.result || {};
        if (result.status === "claimed") {
          setStatus("idle", "Approve in AgentArk");
        }
        if (result.status === "completed" && result.device && result.device_token) {
          clearInterval(retryTimer);
          saveDevice(result.device, result.device_token);
          setStatus("connected", "Paired");
          sendPulse();
        }
        return;
      }
      if (message.type === "auth_ok") {
        if (message.device && saved) {
          saved.display_name = message.device.display_name || saved.display_name;
          localStorage.setItem(storageKey, JSON.stringify(saved));
          renderSaved();
        }
        setStatus("connected", "Online");
        sendPulse();
        return;
      }
      if (message.type === "auth_error" || message.type === "error") {
        setStatus("error", message.error || "Error");
        return;
      }
      if (message.type === "command_dispatch" && message.command) {
        handleCommand(message.command);
      }
    }

    function handleCommand(command) {
      emptyCommands.hidden = true;
      const card = document.createElement("div");
      card.className = "command";
      card.innerHTML = `<div class="command-title">${escapeHtml(command.action || "Command")}</div><div class="command-meta">${escapeHtml(command.capability || "unknown")} - ${escapeHtml(command.id || "")}</div>`;
      const actions = document.createElement("div");
      actions.className = "actions";
      const approve = document.createElement("button");
      approve.className = "primary";
      approve.textContent = command.capability === "notifications" ? "Mark received" : "Approve result";
      const reject = document.createElement("button");
      reject.textContent = "Reject";
      actions.appendChild(approve);
      actions.appendChild(reject);
      card.appendChild(actions);
      commandsEl.prepend(card);

      if (command.capability === "notifications") {
        showNotification(command);
      }

      approve.onclick = () => {
        sendCommandResult(command, true, command.capability === "notifications" ? "Notification received in web companion." : "Approved in web companion.");
        card.remove();
        emptyCommands.hidden = commandsEl.children.length > 0;
      };
      reject.onclick = () => {
        sendCommandResult(command, false, null, "Rejected in web companion.");
        card.remove();
        emptyCommands.hidden = commandsEl.children.length > 0;
      };
    }

    function sendCommandResult(command, success, preview, error) {
      send({
        type: "command_result",
        device_id: saved && saved.device_id,
        command_id: command.id,
        success,
        result_preview: preview || undefined,
        error: error || undefined
      });
    }

    function showNotification(command) {
      if (!("Notification" in window) || Notification.permission !== "granted") {
        return;
      }
      const args = command.arguments || {};
      const title = typeof args.title === "string" ? args.title : "AgentArk";
      const body = typeof args.body === "string" ? args.body : (command.action || "Notification received.");
      try {
        new Notification(title, { body });
      } catch {
        log("Browser notification failed");
      }
    }

    async function allowNotifications() {
      if (!("Notification" in window)) {
        log("Notifications are not available in this browser");
        return;
      }
      const result = await Notification.requestPermission();
      log(`Notification permission: ${result}`);
      sendPulse();
    }

    function clearSaved() {
      localStorage.removeItem(storageKey);
      saved = null;
      renderSaved();
      setStatus("idle", "Device forgotten");
      log("Saved browser companion token removed");
    }

    function init() {
      const query = params();
      wsUrlInput.value = query.get("ws") || defaultWsUrl();
      sessionInput.value = query.get("session_id") || query.get("session") || "";
      codeInput.value = query.get("code") || "";
      document.getElementById("claimButton").onclick = claimPairing;
      document.getElementById("connectButton").onclick = connectSaved;
      document.getElementById("notifyButton").onclick = allowNotifications;
      document.getElementById("clearButton").onclick = clearSaved;
      renderSaved();
      if (sessionInput.value && codeInput.value) {
        setStatus("idle", "Ready to claim");
      }
    }

    init();
  </script>
</body>
</html>
"##;

pub(super) async fn companion_web(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !companion_request_allowed(&state, &headers, addr).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = Html(COMPANION_WEB_HTML).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn companion_ws_url_from_base(base: &str) -> Option<String> {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("https://") {
        return Some(format!("wss://{}/companion/ws", rest));
    }
    None
}

async fn companion_connectivity_payload(state: &AppState) -> serde_json::Value {
    let tunnel = state.tunnel.read().await;
    let websocket_url = tunnel.url.as_deref().and_then(companion_ws_url_from_base);
    serde_json::json!({
        "status": "ok",
        "protocol_version": "agentark-companion-v1",
        "websocket_path": "/companion/ws",
        "tunnel_active": tunnel.active,
        "tunnel_url": tunnel.url.clone(),
        "tunnel_provider": tunnel.provider.as_str(),
        "tunnel_companion_enabled": tunnel.companion_enabled,
        "websocket_url": websocket_url,
        "error": tunnel.error.clone(),
        "xcode_source": "clients/companion/ios",
    })
}

pub(super) async fn get_connectivity(State(state): State<AppState>) -> Response {
    Json(companion_connectivity_payload(&state).await).into_response()
}

fn configured_secret(value: &str) -> bool {
    !value.trim().is_empty() && value.trim() != "[ENCRYPTED]"
}

fn mobile_channel(
    id: &str,
    label: &str,
    kind: &str,
    configured: bool,
    ready: bool,
    detail: impl Into<String>,
    settings_path: &str,
) -> CompanionMobileAccessChannel {
    CompanionMobileAccessChannel {
        id: id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        configured,
        ready,
        detail: detail.into(),
        settings_path: settings_path.to_string(),
    }
}

pub(super) async fn get_mobile_access(State(state): State<AppState>) -> Response {
    let plane = plane_from_state(&state).await;
    let devices = match plane.list_devices().await {
        Ok(devices) => devices,
        Err(error) => {
            return json_error(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            );
        }
    };
    let (config, config_dir) = {
        let agent = state.agent.read().await;
        (agent.config.clone(), agent.config_dir.clone())
    };

    let phone_devices = devices
        .iter()
        .filter(|device| {
            let id = format!("{} {}", device.preset_id, device.platform).to_ascii_lowercase();
            id.contains("ios") || id.contains("iphone") || id.contains("android")
        })
        .collect::<Vec<_>>();
    let online_phone_devices = phone_devices
        .iter()
        .filter(|device| device.state == crate::core::CompanionDeviceState::Online)
        .count();
    let mut phone_capabilities = phone_devices
        .iter()
        .flat_map(|device| {
            let capabilities = if device.available_capabilities.is_empty() {
                &device.token_capabilities
            } else {
                &device.available_capabilities
            };
            capabilities.iter().cloned()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if phone_capabilities.is_empty() && !phone_devices.is_empty() {
        phone_capabilities = vec!["approval_prompt".to_string(), "notifications".to_string()];
    }

    let telegram_ready = config.telegram.as_ref().is_some_and(|telegram| {
        configured_secret(&telegram.bot_token)
            && telegram.allowed_users.len() == 1
            && telegram.allowed_users.first().copied().unwrap_or_default() != 0
    });
    let telegram_configured = config
        .telegram
        .as_ref()
        .is_some_and(|telegram| configured_secret(&telegram.bot_token));

    let (whatsapp_configured, whatsapp_ready) = config
        .whatsapp
        .as_ref()
        .map(|whatsapp| {
            let bridge_ready = match whatsapp.mode {
                crate::channels::whatsapp::WhatsAppMode::CloudApi => {
                    configured_secret(&whatsapp.access_token)
                        && configured_secret(&whatsapp.app_secret)
                        && !whatsapp.phone_number_id.trim().is_empty()
                        && !whatsapp.verify_token.trim().is_empty()
                }
                crate::channels::whatsapp::WhatsAppMode::Baileys => match whatsapp.bridge_runtime()
                {
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => true,
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::External => {
                        !whatsapp.bridge_url.trim().is_empty()
                    }
                },
            };
            (
                bridge_ready,
                bridge_ready
                    && crate::channels::whatsapp::configured_notification_recipient(whatsapp)
                        .is_some(),
            )
        })
        .unwrap_or((false, false));

    let (imessage_configured, imessage_ready) = config
        .imessage
        .as_ref()
        .map(|imessage| {
            let configured =
                configured_secret(&imessage.bridge_token) && !imessage.bridge_url.trim().is_empty();
            (
                configured,
                configured
                    && (!imessage.default_chat_id.trim().is_empty()
                        || !imessage.default_handle.trim().is_empty()),
            )
        })
        .unwrap_or((false, false));

    let twilio_ready = crate::integrations::effective_integration_enabled(&config_dir, "twilio");

    Json(CompanionMobileAccessResponse {
        phone_companion: CompanionPhoneSummary {
            paired: phone_devices.len(),
            online: online_phone_devices,
            capabilities: phone_capabilities,
            can_read_phone_messages: false,
            detail: "Bundled iPhone and Android companions handle AgentArk notifications and approvals only.".to_string(),
        },
        message_channels: vec![
            mobile_channel(
                "telegram",
                "Telegram",
                "chat_channel",
                telegram_configured,
                telegram_ready,
                if telegram_ready {
                    "Ready for phone chat through the Telegram bot."
                } else if telegram_configured {
                    "Telegram token exists, but delivery/user targeting is not ready."
                } else {
                    "Configure a Telegram bot token and allowed user before using Telegram as a phone channel."
                },
                "Settings > Integrations > Channels > Telegram",
            ),
            mobile_channel(
                "whatsapp",
                "WhatsApp",
                "chat_channel",
                whatsapp_configured,
                whatsapp_ready,
                if whatsapp_ready {
                    "Ready for phone chat through the configured WhatsApp channel."
                } else if whatsapp_configured {
                    "WhatsApp bridge/API exists, but no delivery recipient is ready."
                } else {
                    "Configure WhatsApp Cloud API or the WhatsApp bridge before using WhatsApp as a phone channel."
                },
                "Settings > Integrations > Channels > WhatsApp",
            ),
            mobile_channel(
                "imessage",
                "iMessage bridge",
                "macos_bridge",
                imessage_configured,
                imessage_ready,
                if imessage_ready {
                    "Ready through a configured macOS Messages bridge signed into the Apple ID."
                } else if imessage_configured {
                    "iMessage bridge credentials exist, but no default handle/chat target is ready."
                } else {
                    "Requires a macOS Messages bridge signed into the relevant Apple ID; the iPhone companion cannot read iMessage."
                },
                "Settings > Integrations > Channels > iMessage",
            ),
        ],
        sms: mobile_channel(
            "sms",
            "SMS",
            "sms_bridge",
            twilio_ready,
            twilio_ready,
            if twilio_ready {
                "Twilio Voice & SMS is configured. This is Twilio-number SMS, not your iPhone SMS history."
            } else {
                "iPhone companion cannot read SMS. Use Twilio, a carrier bridge, or a custom SMS-capable Android companion."
            },
            "Settings > Integrations > Prebuilt Connectors > Twilio",
        ),
        truth_notes: vec![
            "A connected iPhone companion does not expose personal SMS, iMessage, photos, camera, location, or Shortcuts.".to_string(),
            "Use chat channels when the phone should message AgentArk; use companion devices when AgentArk needs notifications or approval prompts on that device.".to_string(),
            "Companion commands are concrete declared actions and still pass through scoped grants, approval, dispatch, and audit.".to_string(),
        ],
    })
    .into_response()
}

pub(super) async fn start_companion_tunnel(State(state): State<AppState>) -> Response {
    let should_spawn = {
        let tunnel = state.tunnel.read().await;
        !tunnel.active
    };
    if should_spawn {
        if let Err(error) = super::tunnel::spawn_tunnel(&state, None).await {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    }
    {
        let mut tunnel = state.tunnel.write().await;
        tunnel.companion_enabled = true;
    }
    let discovered = super::tunnel::wait_for_tunnel_url(state.tunnel.clone(), 12).await;
    if let Some(url) = discovered.as_deref() {
        super::tunnel::persist_public_tunnel_state(&state, Some(url), None).await;
    }
    Json(companion_connectivity_payload(&state).await).into_response()
}

pub(super) async fn stop_companion_tunnel(State(state): State<AppState>) -> Response {
    let should_stop = {
        let mut tunnel = state.tunnel.write().await;
        tunnel.companion_enabled = false;
        tunnel.active && !tunnel.control_plane_enabled && tunnel.exposed_app_ids.is_empty()
    };
    if should_stop {
        super::tunnel::stop_tunnel_internal(&state).await;
    }
    Json(companion_connectivity_payload(&state).await).into_response()
}

pub(super) async fn list_devices(State(state): State<AppState>) -> Response {
    let plane = plane_from_state(&state).await;
    match (
        plane.list_devices().await,
        plane.overview().await,
        plane.list_pairing_sessions().await,
        plane.list_pending_approval_commands().await,
    ) {
        (Ok(devices), Ok(overview), Ok(pairing_sessions), Ok(pending_approvals)) => {
            Json(serde_json::json!({
                "status": "ok",
                "devices": devices,
                "overview": overview,
                "pairing_sessions": pairing_sessions,
                "pending_approvals": pending_approvals,
            }))
            .into_response()
        }
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => json_error(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        ),
    }
}

pub(super) async fn create_pairing_session(
    State(state): State<AppState>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
    Json(input): Json<crate::core::CompanionPairingSessionCreate>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.create_pairing_session(input, &actor).await {
        Ok(session) => Json(serde_json::json!({
            "status": "ok",
            "session": session,
            "pairing_payload": {
                "protocol_version": "agentark-companion-v1",
                "websocket_path": "/companion/ws",
                "session_id": session.id,
                "code": session.code,
                "expires_at": session.expires_at,
            }
        }))
        .into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn approve_pairing_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.approve_pairing_session(&id, &actor).await {
        Ok(session) => {
            Json(serde_json::json!({ "status": "ok", "session": session })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

async fn current_device_scopes(
    plane: &crate::core::CompanionControlPlane,
    device_id: &str,
) -> Result<Vec<String>, String> {
    let device = plane
        .get_device(device_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "device not found".to_string())?;
    Ok(device.token_capabilities)
}

pub(super) async fn create_command(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<crate::core::CompanionCommandCreate>,
) -> Response {
    let plane = plane_from_state(&state).await;
    let caller_scopes = match current_device_scopes(&plane, &device_id).await {
        Ok(scopes) => scopes,
        Err(error) => return json_error(axum::http::StatusCode::NOT_FOUND, error),
    };
    match plane
        .create_command(&device_id, input, &caller_scopes)
        .await
    {
        Ok(command) => {
            Json(serde_json::json!({ "status": "ok", "command": command })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn list_commands(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Response {
    let plane = plane_from_state(&state).await;
    match plane.list_commands(&device_id).await {
        Ok(commands) => {
            Json(serde_json::json!({ "status": "ok", "commands": commands })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CommandApprovalPayload {
    #[serde(default = "default_true")]
    approved: bool,
    #[serde(default)]
    reason: Option<String>,
}

pub(super) async fn approve_command(
    State(state): State<AppState>,
    Path(command_id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
    Json(input): Json<CommandApprovalPayload>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane
        .approve_command(&command_id, &actor, input.approved, input.reason)
        .await
    {
        Ok(command) => {
            Json(serde_json::json!({ "status": "ok", "command": command })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn revoke_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    maybe_caller: Option<axum::extract::Extension<crate::actions::ActionCallerPrincipal>>,
) -> Response {
    let actor = actor_label(maybe_caller.as_ref().map(|ext| &ext.0), "ui");
    let plane = plane_from_state(&state).await;
    match plane.revoke_device(&device_id, &actor).await {
        Ok(device) => Json(serde_json::json!({ "status": "ok", "device": device })).into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn rotate_token(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    Json(input): Json<crate::core::CompanionTokenRotationRequest>,
) -> Response {
    let plane = plane_from_state(&state).await;
    let caller_scopes = match current_device_scopes(&plane, &device_id).await {
        Ok(scopes) => scopes,
        Err(error) => return json_error(axum::http::StatusCode::NOT_FOUND, error),
    };
    match plane
        .rotate_token(&device_id, input.requested_scopes, &caller_scopes)
        .await
    {
        Ok(result) => {
            Json(serde_json::json!({ "status": "ok", "rotation": result })).into_response()
        }
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct AuditQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn get_audit(
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> Response {
    let plane = plane_from_state(&state).await;
    match plane.list_audit_events(query.limit.unwrap_or(100)).await {
        Ok(events) => Json(serde_json::json!({ "status": "ok", "events": events })).into_response(),
        Err(error) => json_error(axum::http::StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub(super) async fn companion_ws(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !companion_request_allowed(&state, &headers, addr).await {
        if companion_request_is_active_non_https_tunnel(&state, &headers).await {
            return json_error(
                StatusCode::UPGRADE_REQUIRED,
                "AgentArk Companion requires secure AgentArk remote access.",
            );
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    let initial_auth = companion_ws_header_auth(&headers);
    ws.on_upgrade(move |socket| handle_companion_socket(state, socket, initial_auth))
}

#[derive(Debug, Deserialize)]
struct CompanionWsEnvelope {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    device_public_key: Option<String>,
    #[serde(default)]
    attestation: Option<crate::core::connectivity::companion::CompanionAttestationClaim>,
    #[serde(default)]
    state: Option<crate::core::CompanionDeviceState>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    commands: Vec<crate::core::connectivity::companion::CompanionCommandDescriptor>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    command_id: Option<String>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    result_preview: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn send_ws_json(
    sender: &mut futures::stream::SplitSink<WebSocket, AxumWsMessage>,
    payload: serde_json::Value,
) -> bool {
    let Ok(text) = serde_json::to_string(&payload) else {
        return false;
    };
    sender.send(AxumWsMessage::Text(text.into())).await.is_ok()
}

async fn send_next_command(
    plane: &crate::core::CompanionControlPlane,
    sender: &mut futures::stream::SplitSink<WebSocket, AxumWsMessage>,
    device_id: &str,
) -> bool {
    match plane.dispatch_next_command(device_id).await {
        Ok(Some(command)) => {
            send_ws_json(
                sender,
                serde_json::json!({
                    "type": "command_dispatch",
                    "command": command,
                }),
            )
            .await
        }
        Ok(None) => true,
        Err(error) => {
            send_ws_json(
                sender,
                serde_json::json!({
                    "type": "error",
                    "error": error.to_string(),
                }),
            )
            .await
        }
    }
}

fn companion_ws_device_id_allowed(message_device_id: Option<&str>, authed_device_id: &str) -> bool {
    match message_device_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(message_device_id) => message_device_id == authed_device_id,
        None => true,
    }
}

async fn handle_companion_socket(
    state: AppState,
    socket: WebSocket,
    initial_auth: Option<CompanionWsAuth>,
) {
    let plane = plane_from_state(&state).await;
    let (mut sender, mut receiver) = socket.split();
    let mut authed_device_id: Option<String> = None;
    let _ = send_ws_json(
        &mut sender,
        serde_json::json!({
            "type": "hello",
            "protocol_version": "agentark-companion-v1",
            "pairing_required": true,
            "auth_transport": "authorization_header",
            "browser_auth_message": "browser_auth",
        }),
    )
    .await;
    if let Some(auth) = initial_auth {
        match plane
            .verify_device_token(&auth.device_id, &auth.token)
            .await
        {
            Ok(device) => {
                authed_device_id = Some(device.id.clone());
                let _ = plane
                    .pulse_device(
                        &device.id,
                        Some(crate::core::CompanionDeviceState::Online),
                        Vec::new(),
                        Vec::new(),
                        BTreeMap::new(),
                    )
                    .await;
                let _ = send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "auth_ok",
                        "device": device,
                    }),
                )
                .await;
                let _ = send_next_command(&plane, &mut sender, &auth.device_id).await;
            }
            Err(error) => {
                let _ = send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "auth_error",
                        "error": error.to_string(),
                    }),
                )
                .await;
            }
        }
    }

    while let Some(next) = receiver.next().await {
        let Ok(message) = next else {
            break;
        };
        let AxumWsMessage::Text(text) = message else {
            continue;
        };
        let parsed = match serde_json::from_str::<CompanionWsEnvelope>(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                if !send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "error",
                        "error": format!("invalid companion message: {error}"),
                    }),
                )
                .await
                {
                    break;
                }
                continue;
            }
        };

        match parsed.message_type.as_str() {
            "pairing_claim" => {
                let claim = crate::core::CompanionPairingClaim {
                    session_id: parsed.session_id.unwrap_or_default(),
                    code: parsed.code.unwrap_or_default(),
                    device_public_key: parsed.device_public_key,
                    attestation: parsed.attestation,
                    metadata: parsed.metadata,
                };
                match plane.claim_pairing_session(claim).await {
                    Ok(result) => {
                        if let Some(device) = result.device.as_ref() {
                            authed_device_id = Some(device.id.clone());
                        }
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "pairing_claim_result",
                                "result": result,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if let Some(device_id) = authed_device_id.clone() {
                            if !send_next_command(&plane, &mut sender, &device_id).await {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            "browser_auth" => {
                let device_id = parsed.device_id.unwrap_or_default();
                let token = parsed.token.unwrap_or_default();
                match plane.verify_device_token(&device_id, &token).await {
                    Ok(device) => {
                        authed_device_id = Some(device.id.clone());
                        let _ = plane
                            .pulse_device(
                                &device.id,
                                Some(crate::core::CompanionDeviceState::Online),
                                Vec::new(),
                                Vec::new(),
                                BTreeMap::new(),
                            )
                            .await;
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "auth_ok",
                                "device": device,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if !send_next_command(&plane, &mut sender, &device_id).await {
                            break;
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "auth_error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            "auth" => {
                if parsed.token.is_some() {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "send companion tokens in the WebSocket Authorization header, not in JSON messages",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "missing WebSocket Authorization header",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                if let Some(device) = plane.get_device(&device_id).await.ok().flatten() {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "auth_ok",
                            "device": device,
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    if !send_next_command(&plane, &mut sender, &device_id).await {
                        break;
                    }
                }
            }
            "pulse" | "capability_report" => {
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "authenticate before pulse or capability_report",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                match plane
                    .pulse_device(
                        &device_id,
                        parsed.state,
                        parsed.capabilities,
                        parsed.commands,
                        parsed.metadata,
                    )
                    .await
                {
                    Ok(device) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "pulse_ok",
                                "device": device,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                        if !send_next_command(&plane, &mut sender, &device_id).await {
                            break;
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            "command_result" => {
                let Some(device_id) = authed_device_id.clone() else {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "authenticate before command_result",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                };
                if !companion_ws_device_id_allowed(parsed.device_id.as_deref(), &device_id) {
                    if !send_ws_json(
                        &mut sender,
                        serde_json::json!({
                            "type": "error",
                            "error": "message device_id does not match the authenticated device",
                        }),
                    )
                    .await
                    {
                        break;
                    }
                    continue;
                }
                match plane
                    .complete_command(
                        &device_id,
                        parsed.command_id.as_deref().unwrap_or_default(),
                        parsed.success.unwrap_or(false),
                        parsed.result_preview,
                        parsed.error,
                    )
                    .await
                {
                    Ok(command) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "command_result_ok",
                                "command": command,
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        if !send_ws_json(
                            &mut sender,
                            serde_json::json!({
                                "type": "error",
                                "error": error.to_string(),
                            }),
                        )
                        .await
                        {
                            break;
                        }
                    }
                }
            }
            _ => {
                if !send_ws_json(
                    &mut sender,
                    serde_json::json!({
                        "type": "error",
                        "error": "unsupported companion protocol message type",
                    }),
                )
                .await
                {
                    break;
                }
            }
        }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_websocket_url_is_advertised_only_for_https_tunnels() {
        assert_eq!(
            companion_ws_url_from_base("https://agentark.example"),
            Some("wss://agentark.example/companion/ws".to_string())
        );
        assert_eq!(companion_ws_url_from_base("http://agentark.example"), None);
    }
}
