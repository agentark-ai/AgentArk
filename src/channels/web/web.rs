//! Web UI channel assets.
//!
//! The full application UI is served from frontend assets.
//! This module only keeps locked-mode HTML for master-password unlock.

/// Unlock page HTML - shown when master password is required
const UNLOCK_PAGE_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>__PRODUCT_NAME__ - Unlock</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #0a0a1a 0%, #1a1a2e 50%, #16213e 100%);
            color: #e0e0e0;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
        }
        .unlock-card {
            background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.1);
            border-radius: 16px;
            padding: 44px 38px 38px;
            max-width: 420px;
            width: 90%;
            text-align: center;
            backdrop-filter: blur(20px);
        }
        .unlock-card img {
            width: clamp(84px, 12vw, 104px);
            height: clamp(84px, 12vw, 104px);
            margin-bottom: 20px;
            filter: drop-shadow(0 8px 20px rgba(108, 92, 231, 0.28));
        }
        .unlock-card h1 { font-size: 1.4em; margin-bottom: 8px; color: #fff; }
        .unlock-card p { font-size: 0.85em; color: #999; margin-bottom: 24px; }
        .unlock-card input {
            width: 100%;
            padding: 12px 16px;
            background: rgba(255,255,255,0.08);
            border: 1px solid rgba(255,255,255,0.15);
            border-radius: 8px;
            color: #fff;
            font-size: 0.95em;
            outline: none;
            margin-bottom: 16px;
        }
        .unlock-card input:focus { border-color: #6c5ce7; }
        .unlock-card button {
            width: 100%;
            padding: 12px;
            background: linear-gradient(135deg, #6c5ce7, #a855f7);
            border: none;
            border-radius: 8px;
            color: #fff;
            font-size: 0.95em;
            font-weight: 600;
            cursor: pointer;
        }
        .unlock-card button:hover { opacity: 0.9; }
        .unlock-card button:disabled { opacity: 0.5; cursor: wait; }
        .error { color: #ff6b6b; font-size: 0.82em; margin-top: 12px; }
        .success { color: #51cf66; font-size: 0.82em; margin-top: 12px; }
        .hint {
            font-size: 0.75em; color: #666; margin-top: 20px;
            border-top: 1px solid rgba(255,255,255,0.05); padding-top: 16px;
        }
    </style>
</head>
<body>
    <div class="unlock-card">
        <img src="/logo.svg" alt="__PRODUCT_NAME__">
        <h1>__PRODUCT_NAME__ is Locked</h1>
        <p>Enter your master password to unlock the agent.</p>
        <form id="unlock-form">
            <input type="password" id="password" placeholder="Master password"
                   autofocus autocomplete="current-password">
            <button type="submit" id="unlock-btn">Unlock</button>
            <div id="msg" style="display:none"></div>
        </form>
        <div class="hint">
            Enter your master password to unlock __PRODUCT_NAME__.
        </div>
    </div>
    <script>
        const DEFAULT_NEXT_TARGET = "__AGENTARK_NEXT_TARGET__";

        function computeNextTarget() {
            try {
                const url = new URL(window.location.href);
                const requested = url.searchParams.get('next');
                if (requested && requested.startsWith('/')) {
                    return requested;
                }
            } catch (_err) {
                // Fall through to path-based detection below.
            }

            const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
            if (window.location.pathname === '/' || window.location.pathname.startsWith('/ui')) {
                return current || '/';
            }
            return DEFAULT_NEXT_TARGET || '/';
        }

        async function waitForUnlockedStartup() {
            const deadline = Date.now() + 90000;
            while (Date.now() < deadline) {
                try {
                    const res = await fetch('/health', { cache: 'no-store' });
                    if (res.ok) {
                        const data = await res.json();
                        if (data && data.status && data.status !== 'locked') {
                            return true;
                        }
                    }
                } catch (_err) {
                    // Full server may still be restarting; keep polling.
                }
                await new Promise((resolve) => setTimeout(resolve, 1000));
            }
            return false;
        }

        document.getElementById('unlock-form').onsubmit = async (e) => {
            e.preventDefault();
            const btn = document.getElementById('unlock-btn');
            const msg = document.getElementById('msg');
            const pw = document.getElementById('password').value;
            if (!pw) return;
            btn.disabled = true;
            btn.textContent = 'Unlocking...';
            msg.style.display = 'none';
            try {
                const res = await fetch('/unlock', {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({password: pw})
                });
                const data = await res.json();
                if (res.ok) {
                    msg.className = 'success';
                    msg.textContent = 'Unlocked! Waiting for __PRODUCT_NAME__ to finish starting...';
                    msg.style.display = 'block';
                    btn.textContent = 'Starting...';
                    const nextTarget = computeNextTarget();
                    const ready = await waitForUnlockedStartup();
                    if (ready) {
                        window.location.href = nextTarget;
                        return;
                    }
                    msg.className = 'error';
                    msg.textContent = 'Password accepted, but __PRODUCT_NAME__ is still starting. Refresh in a few seconds.';
                    btn.disabled = false;
                    btn.textContent = 'Unlock';
                } else {
                    msg.className = 'error';
                    msg.textContent = data.error || 'Invalid password';
                    msg.style.display = 'block';
                    btn.disabled = false;
                    btn.textContent = 'Unlock';
                    document.getElementById('password').select();
                }
            } catch(err) {
                msg.className = 'error';
                msg.textContent = 'Connection error';
                msg.style.display = 'block';
                btn.disabled = false;
                btn.textContent = 'Unlock';
            }
        };
    </script>
</body>
</html>
"##;

pub fn render_unlock_page_html(next_target: &str) -> String {
    let sanitized_target = if next_target.starts_with('/') {
        next_target
    } else {
        "/"
    };
    let next_target_json =
        serde_json::to_string(sanitized_target).unwrap_or_else(|_| "\"/\"".to_string());
    crate::branding::render_template(UNLOCK_PAGE_HTML_TEMPLATE)
        .replace("\"__AGENTARK_NEXT_TARGET__\"", &next_target_json)
}
