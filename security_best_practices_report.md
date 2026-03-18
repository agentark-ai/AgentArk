# Security Best Practices Report

## Executive Summary

- Overall status: Red
- Scope: Static security review of the Rust/Axum backend, React/Vite frontend, Node sidecars, and the FastAPI mem0 bridge
- Constraint: No `cargo` builds, tests, or runtime verification were performed
- Highest-risk issue: the public UI bootstrap path issues a valid authenticated session cookie to any visitor, which collapses the API-key boundary for the protected control plane

Top findings:

1. SEC-001 Critical: Anonymous requests to `/` and `/ui` receive a valid `agentark_session` cookie that the auth middleware accepts for protected routes
2. SEC-002 High: `/oauth/callback` reflects attacker-controlled query data directly into HTML, enabling same-origin XSS
3. SEC-003 High: OAuth flows use fixed `state` values such as `gmail` instead of a per-request nonce, so the callback is vulnerable to login CSRF / account binding attacks
4. SEC-004 Medium: the Playwright sidecar exposes unauthenticated browser control and raw page JavaScript execution while binding to `0.0.0.0`

## Stack Context

- Backend: Rust 2021, Axum, Tokio, SeaORM, SQLite
- Frontend: React 18, TypeScript, Vite
- Sidecars: Node/Express Playwright bridge, Node/Express WhatsApp bridge, FastAPI mem0 bridge
- Deployment: Docker Compose, main HTTP service published on port `8990`
- Note: the `security-best-practices` skill has direct reference coverage for the React and Python portions of the repo; the Rust backend review below is manual

## Findings

### SEC-001 - Public UI bootstraps an authenticated control-plane session for every visitor

- Severity: Critical
- Impact: any client that can reach the AgentArk HTTP port can become authenticated by first loading `/` or `/ui`, then call protected endpoints such as secret reveal, API-key rotation, tunnel management, and master-password changes
- Locations:
  - `src/channels/http.rs:3871-3900`
  - `src/channels/http.rs:3910-3921`
  - `src/channels/http.rs:3757-3760`
  - `src/channels/http.rs:4551-4555`
  - `src/channels/http.rs:5546-5555`
  - `src/channels/http.rs:4032-4035`
  - `src/channels/http.rs:4207-4212`
  - `src/channels/http.rs:15401-15418`

- Evidence:
  - A session token is created whenever an API key exists:

    ```rust
    // src/channels/http.rs:3869-3876
    let session_token = initial_api_key.as_ref().map(|_| {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
    });
    ```

  - The routes serving `/`, `/ui`, and `/ui/v2` are explicitly public:

    ```rust
    // src/channels/http.rs:3909-3913
    let public_routes = Router::new()
        .route("/", get(web_ui))
        .route("/ui", get(web_ui))
        .route("/ui/v2", get(web_ui_v2))
    ```

  - The public UI handlers always attach the session cookie:

    ```rust
    // src/channels/http.rs:4551-4555
    apply_session_cookie(
        &mut response,
        state.session_token.as_ref(),
        state.cookie_secure_default || is_https_forwarded(&headers),
    );
    ```

  - The auth middleware accepts that cookie as sufficient authentication for protected routes:

    ```rust
    // src/channels/http.rs:3757-3760
    if has_valid_ui_session_cookie(request.headers(), state.session_token.as_deref()) {
        return next.run(request).await;
    }
    ```

  - The cookie is a full-site auth cookie:

    ```rust
    // src/channels/http.rs:5546-5555
    "agentark_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=86400{}"
    ```

  - Sensitive protected routes include secret management and security administration:

    ```rust
    // src/channels/http.rs:4032-4035
    .route("/settings/secrets", get(list_settings_secrets))
    .route("/settings/secrets/reveal", post(reveal_settings_secrets))
    .route("/settings/secrets/upsert", post(upsert_settings_secret))
    .route("/settings/secrets/delete", post(delete_settings_secret))
    ```

    ```rust
    // src/channels/http.rs:4209-4212
    .route("/security/status", get(security_status))
    .route("/security/set-password", post(set_master_password))
    .route("/security/change-password", post(change_master_password))
    .route("/security/remove-password", post(remove_master_password))
    ```

- Why this matters:
  - The API-key guard is effectively bypassed by design for any network client able to visit the public UI.
  - Once the cookie is issued, protected routes are reachable without a bearer token.
  - This is especially dangerous when `AGENTARK_BIND=0.0.0.0:8990` is used in Docker Compose.

- Recommended fix:
  - Stop minting authenticated cookies from anonymous public routes.
  - Require an explicit login/bootstrap step before issuing any session cookie.
  - Bind that session to a server-side session store or a signed, scoped token that is only issued after bearer-key validation or successful unlock.
  - Add a regression test that proves `GET /` does not grant access to protected routes.

### SEC-002 - OAuth callback reflects attacker-controlled data into HTML

- Severity: High
- Impact: an attacker can run JavaScript in the AgentArk origin through `/oauth/callback`, then use same-origin requests to operate the control plane
- Locations:
  - `src/channels/http.rs:18365-18389`
  - `src/channels/http.rs:18399-18420`
  - `src/channels/http.rs:18423-18447`
  - `src/channels/http.rs:18450-18474`

- Evidence:
  - The error callback path injects the attacker-controlled `error` query parameter into HTML:

    ```rust
    // src/channels/http.rs:18365-18389
    if let Some(error) = params.error {
        let service_id = params.state.unwrap_or_else(|| "unknown".to_string());
        let signal = oauth_callback_signal_script(&service_id, "error", &error);
        let html = format!(r#"...<p>{}</p>...{}..."#, error, signal);
    }
    ```

  - The success path injects `state`-derived `service_id` into HTML:

    ```rust
    // src/channels/http.rs:18399-18420
    let service_id = params.state.unwrap_or_else(|| "unknown".to_string());
    let result = match service_id.as_str() {
        "gmail" => ...
        "google_calendar" | "calendar" => ...
        "whatsapp" => ...
        _ => Err(anyhow::anyhow!("Unknown service: {}", service_id)),
    };
    ```

    ```rust
    // src/channels/http.rs:18423-18447
    let html = format!(r#"...<h2 class="success">... {} Connected!</h2>...{}..."#,
        service_id.replace('_', " "),
        signal
    );
    ```

  - The error-rendering path also reflects provider error text without escaping:

    ```rust
    // src/channels/http.rs:18450-18474
    let error_text = e.to_string();
    let html = format!(r#"...<p>{}</p>...{}..."#, error_text, signal);
    ```

- Why this matters:
  - `/oauth/callback` is a public route.
  - A malicious page or local lure can target `http://localhost:8990/oauth/callback?...` and inject HTML/JS.
  - Once script executes in the AgentArk origin, it can fetch `/ui` to receive the session cookie from SEC-001 and then call protected endpoints.

- Recommended fix:
  - HTML-escape all interpolated values before rendering.
  - Prefer rendering a static callback page and pass structured JSON through a safe script literal or DOM text node instead of string interpolation.
  - Add a restrictive CSP at the app or edge as defense in depth, but do not treat CSP as the primary fix.

### SEC-003 - OAuth flows do not use a per-request CSRF state token

- Severity: High
- Impact: an attacker can bind their own Google account to a victim's local AgentArk instance by driving the victim browser through the callback with an attacker-controlled authorization code
- Locations:
  - `src/channels/http.rs:14975-14984`
  - `src/channels/http.rs:17979-18001`
  - `src/channels/http.rs:18399-18420`

- Evidence:
  - Gmail auth starts with a fixed `state=gmail`:

    ```rust
    // src/channels/http.rs:14975-14984
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?...&state=gmail&access_type=offline&prompt=consent",
        ...
    );
    ```

  - Calendar auth also uses a fixed state string:

    ```rust
    // src/channels/http.rs:17986-18001
    Ok(format!(
        "https://accounts.google.com/o/oauth2/v2/auth?...&state=calendar&access_type=offline&prompt=consent",
        ...
    ))
    ```

  - The callback trusts `params.state` only as a service selector and does not validate a nonce bound to the initiating user session:

    ```rust
    // src/channels/http.rs:18399-18420
    let service_id = params.state.unwrap_or_else(|| "unknown".to_string());
    let result = match service_id.as_str() {
        "gmail" => gmail_exchange_code(&state, &code).await,
        "google_calendar" | "calendar" => calendar_exchange_code(&state, &code).await,
        ...
    };
    ```

- Why this matters:
  - OAuth `state` is supposed to be an unpredictable request-binding token, not a constant label.
  - Because the callback is public and local-browser reachable, a malicious website can trigger a GET to the callback endpoint with an attacker's code.
  - The server will exchange the code and store tokens without confirming that the same browser session initiated the flow.

- Recommended fix:
  - Generate a cryptographically random `state` per OAuth initiation.
  - Store it server-side with the target integration, an expiry, and the initiating session.
  - Reject the callback unless the supplied state matches and consume it once.
  - Add PKCE where the provider supports it.

### SEC-004 - Playwright bridge exposes full browser control without authentication and binds all interfaces

- Severity: Medium
- Impact: any process or container that can reach port `3100` can create browser sessions, navigate to arbitrary URLs, take screenshots, scrape content, and execute arbitrary page JavaScript
- Locations:
  - `services/playwright-bridge/index.js:47-60`
  - `services/playwright-bridge/index.js:75-83`
  - `services/playwright-bridge/index.js:184-226`
  - `services/playwright-bridge/index.js:233-240`
  - `services/playwright-bridge/index.js:282-283`
  - `docker-entrypoint.sh:101-107`

- Evidence:
  - The service exposes session creation and navigation with no auth layer:

    ```javascript
    // services/playwright-bridge/index.js:47-60
    app.post('/session', async (req, res) => { ... });
    ```

    ```javascript
    // services/playwright-bridge/index.js:75-83
    app.post('/session/:id/navigate', async (req, res) => {
      const { url } = req.body;
      await session.page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 });
    });
    ```

  - It exposes content scraping and raw JS evaluation:

    ```javascript
    // services/playwright-bridge/index.js:184-226
    app.get('/session/:id/content', async (req, res) => { ... });
    ```

    ```javascript
    // services/playwright-bridge/index.js:233-240
    app.post('/session/:id/evaluate', async (req, res) => {
      const { expression } = req.body;
      const result = await session.page.evaluate(expression);
    });
    ```

  - The bridge binds `0.0.0.0`, even though the entrypoint comments describe it as localhost-only:

    ```javascript
    // services/playwright-bridge/index.js:282-283
    app.listen(PORT, '0.0.0.0', () => {
      console.log(`Playwright bridge listening on port ${PORT}`);
    });
    ```

    ```bash
    # docker-entrypoint.sh:101-107
    # Start Playwright bridge in background (localhost-only)
    PLAYWRIGHT_BROWSERS_PATH=${PLAYWRIGHT_BROWSERS_PATH:-/ms-playwright} \
    PORT=${PLAYWRIGHT_BRIDGE_PORT:-3100} \
    gosu agent node /app/playwright-bridge/index.js &
    ```

- Why this matters:
  - The entrypoint never constrains the bind host, so the service listens on every interface inside the container.
  - Even if port `3100` is not published on the host, the service becomes reachable to any co-resident process or reachable container that can address the AgentArk container.
  - The `/evaluate` endpoint turns that into a high-privilege browser automation channel.

- Recommended fix:
  - Bind the bridge explicitly to `127.0.0.1`.
  - Require a shared bearer token or Unix socket style trust boundary for every bridge route.
  - Remove or heavily restrict `/evaluate` unless it is strictly necessary.
  - Add an integration test that proves the bridge rejects unauthenticated requests.

## Additional Hardening Gaps

- No browser security header configuration such as CSP was visible in the repo. That may exist at a reverse proxy or tunnel edge, but it is not visible in application code. Given SEC-002, verify this explicitly at runtime.
- The review did not include runtime traffic inspection, dependency audit execution, or fuzzing because the request was read-only and no build/run steps were allowed.

## Recommended Fix Order

1. Fix SEC-001 before any production or remote exposure. This is a control-plane authentication bypass.
2. Fix SEC-002 immediately after. It provides same-origin script execution on a public route.
3. Fix SEC-003 next. It allows cross-session account binding through OAuth callbacks.
4. Fix SEC-004 as defense in depth for the container-internal browser automation plane.

## High-Value Tests To Add

1. Assert that `GET /` and `GET /ui` do not yield a session that can call `/settings/secrets`, `/settings/secrets/reveal`, or `/security/set-password`.
2. Add a callback rendering test that injects HTML in `error` and `state` and verifies the response escapes it.
3. Add an OAuth flow test that fails when the callback `state` does not match a stored nonce.
4. Add a Playwright bridge test that rejects unauthenticated access to `/session` and `/session/:id/evaluate`.

