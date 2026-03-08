/**
 * AgentArk WhatsApp Bridge — lightweight Baileys-based WhatsApp Web connector.
 *
 * Connects to WhatsApp via multi-device Web API. No Meta Business account needed.
 * Users scan a QR code to link their phone.
 *
 * ENV vars:
 *   AGENTARK_URL      – AgentArk webhook URL (default: http://agentark:8990)
 *   AGENTARK_API_KEY  – Bearer token for AgentArk API
 *   BRIDGE_PORT       – Port this bridge listens on (default: 3100)
 *   AUTH_DIR          – Directory to persist auth state (default: /data/auth)
 */

const {
  default: makeWASocket,
  useMultiFileAuthState,
  DisconnectReason,
  fetchLatestBaileysVersion,
  makeCacheableSignalKeyStore,
} = require("@whiskeysockets/baileys");
const express = require("express");
const QRCode = require("qrcode");
const pino = require("pino");
const fs = require("fs");
const path = require("path");

const logger = pino({ level: "info" });

const AGENTARK_URL = process.env.AGENTARK_URL || "http://127.0.0.1:8990";
const AGENTARK_API_KEY = process.env.AGENTARK_API_KEY || "";
const BRIDGE_PORT = parseInt(process.env.BRIDGE_PORT || "8999", 10);
const BRIDGE_HOST = process.env.BRIDGE_HOST || "127.0.0.1";
const AUTH_DIR = process.env.AUTH_DIR || "/app/data/whatsapp-auth";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let sock = null;
let currentQR = null; // base64 data-URI of current QR code
let connectionStatus = "disconnected"; // disconnected | qr | connecting | connected
let connectedNumber = null; // e.g. "15551234567"
let reconnectTimer = null;

// ---------------------------------------------------------------------------
// Express API
// ---------------------------------------------------------------------------

const app = express();
app.use(express.json());

/** Health check */
app.get("/health", (_req, res) => {
  res.json({ status: "ok", connection: connectionStatus, number: connectedNumber });
});

/** Get current status + QR code (if pending) */
app.get("/status", (_req, res) => {
  res.json({
    status: connectionStatus,
    number: connectedNumber,
    qr: currentQR || null,
  });
});

/** QR code as an image (for embedding in UI) */
app.get("/qr", (_req, res) => {
  if (!currentQR) {
    return res.status(404).json({ error: "No QR code available", status: connectionStatus });
  }
  // Return the QR as a PNG data URI in JSON (easy to embed in <img>)
  res.json({ qr: currentQR, status: connectionStatus });
});

/** QR code as raw PNG image */
app.get("/qr.png", (_req, res) => {
  if (!currentQR) {
    return res.status(404).send("No QR code available");
  }
  const base64 = currentQR.replace(/^data:image\/png;base64,/, "");
  const buf = Buffer.from(base64, "base64");
  res.set("Content-Type", "image/png");
  res.send(buf);
});

/** Send a text message */
app.post("/send", async (req, res) => {
  const { to, text } = req.body;
  if (!to || !text) {
    return res.status(400).json({ error: "Missing 'to' or 'text' field" });
  }
  if (!sock || connectionStatus !== "connected") {
    return res.status(503).json({ error: "WhatsApp not connected" });
  }

  try {
    // Normalize number: ensure it has @s.whatsapp.net suffix
    const jid = to.includes("@") ? to : `${to}@s.whatsapp.net`;

    // Split long messages (WhatsApp limit ~4096 chars)
    const MAX_LEN = 4000;
    const chunks = [];
    if (text.length <= MAX_LEN) {
      chunks.push(text);
    } else {
      let remaining = text;
      while (remaining.length > 0) {
        let cut = Math.min(MAX_LEN, remaining.length);
        // Try to cut at a newline
        if (cut < remaining.length) {
          const nl = remaining.lastIndexOf("\n", cut);
          if (nl > cut * 0.5) cut = nl + 1;
        }
        chunks.push(remaining.slice(0, cut));
        remaining = remaining.slice(cut);
      }
    }

    for (const chunk of chunks) {
      await sock.sendMessage(jid, { text: chunk });
    }

    res.json({ ok: true, chunks: chunks.length });
  } catch (err) {
    logger.error({ err }, "Failed to send message");
    res.status(500).json({ error: err.message });
  }
});

/** Send composing (typing) presence to a chat */
app.post("/presence", async (req, res) => {
  const { to, type: presenceType } = req.body;
  if (!to) {
    return res.status(400).json({ error: "Missing 'to' field" });
  }
  if (!sock || connectionStatus !== "connected") {
    return res.status(503).json({ error: "WhatsApp not connected" });
  }
  try {
    const jid = to.includes("@") ? to : `${to}@s.whatsapp.net`;
    await sock.sendPresenceUpdate(presenceType || "composing", jid);
    res.json({ ok: true });
  } catch (err) {
    logger.error({ err }, "Failed to send presence");
    res.status(500).json({ error: err.message });
  }
});

/** Send a video message (base64-encoded) */
app.post("/send-video", async (req, res) => {
  const { to, video, caption } = req.body;
  if (!to || !video) {
    return res.status(400).json({ error: "Missing 'to' or 'video' (base64) field" });
  }
  if (!sock || connectionStatus !== "connected") {
    return res.status(503).json({ error: "WhatsApp not connected" });
  }

  try {
    const jid = to.includes("@") ? to : `${to}@s.whatsapp.net`;
    const buffer = Buffer.from(video, "base64");

    await sock.sendMessage(jid, {
      video: buffer,
      caption: caption || "",
      mimetype: "video/mp4",
    });

    res.json({ ok: true });
  } catch (err) {
    logger.error({ err }, "Failed to send video");
    res.status(500).json({ error: err.message });
  }
});

/** Send an image message (base64-encoded) */
app.post("/send-image", async (req, res) => {
  const { to, image, caption } = req.body;
  if (!to || !image) {
    return res.status(400).json({ error: "Missing 'to' or 'image' (base64) field" });
  }
  if (!sock || connectionStatus !== "connected") {
    return res.status(503).json({ error: "WhatsApp not connected" });
  }

  try {
    const jid = to.includes("@") ? to : `${to}@s.whatsapp.net`;
    const buffer = Buffer.from(image, "base64");

    await sock.sendMessage(jid, {
      image: buffer,
      caption: caption || "",
      mimetype: "image/jpeg",
    });

    res.json({ ok: true });
  } catch (err) {
    logger.error({ err }, "Failed to send image");
    res.status(500).json({ error: err.message });
  }
});

/** Disconnect and clear auth (re-pair) */
app.post("/logout", async (_req, res) => {
  try {
    if (sock) {
      await sock.logout();
    }
    // Clear auth files
    if (fs.existsSync(AUTH_DIR)) {
      fs.rmSync(AUTH_DIR, { recursive: true, force: true });
    }
    connectionStatus = "disconnected";
    connectedNumber = null;
    currentQR = null;

    // Reconnect (will show new QR)
    setTimeout(startConnection, 1000);
    res.json({ ok: true, message: "Logged out. Scan QR again to reconnect." });
  } catch (err) {
    logger.error({ err }, "Logout failed");
    res.status(500).json({ error: err.message });
  }
});

// ---------------------------------------------------------------------------
// Baileys Connection
// ---------------------------------------------------------------------------

async function startConnection() {
  // Ensure auth dir exists
  fs.mkdirSync(AUTH_DIR, { recursive: true });

  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();

  logger.info({ version }, "Starting Baileys connection");

  sock = makeWASocket({
    version,
    auth: {
      creds: state.creds,
      keys: makeCacheableSignalKeyStore(state.keys, logger),
    },
    logger: pino({ level: "warn" }),
    generateHighQualityLinkPreview: false,
  });

  // ---- QR code event ----
  sock.ev.on("connection.update", async (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      logger.info("QR code received — waiting for scan");
      connectionStatus = "qr";
      try {
        currentQR = await QRCode.toDataURL(qr, { width: 300, margin: 2 });
      } catch (e) {
        logger.error({ e }, "Failed to generate QR data URI");
      }
    }

    if (connection === "open") {
      connectionStatus = "connected";
      currentQR = null;
      // Extract own phone number
      const me = sock.user;
      connectedNumber = me?.id?.split(":")[0] || me?.id?.split("@")[0] || null;
      logger.info({ number: connectedNumber }, "WhatsApp connected");
    }

    if (connection === "close") {
      const statusCode = lastDisconnect?.error?.output?.statusCode;
      const shouldReconnect = statusCode !== DisconnectReason.loggedOut;

      logger.info({ statusCode, shouldReconnect }, "Connection closed");

      connectionStatus = "disconnected";
      connectedNumber = null;
      currentQR = null;

      if (shouldReconnect) {
        logger.info("Reconnecting in 3 seconds...");
        if (reconnectTimer) clearTimeout(reconnectTimer);
        reconnectTimer = setTimeout(startConnection, 3000);
      } else {
        logger.info("Logged out — clear auth to re-pair");
        // Clear auth on explicit logout
        if (fs.existsSync(AUTH_DIR)) {
          fs.rmSync(AUTH_DIR, { recursive: true, force: true });
        }
      }
    }
  });

  // ---- Persist credentials ----
  sock.ev.on("creds.update", saveCreds);

  // ---- Incoming messages ----
  sock.ev.on("messages.upsert", async ({ messages, type }) => {
    if (type !== "notify") return;

    for (const msg of messages) {
      // Skip own messages and status broadcasts
      if (msg.key.fromMe) continue;
      if (msg.key.remoteJid === "status@broadcast") continue;

      const from = msg.key.remoteJid?.split("@")[0] || "";
      const messageId = msg.key.id || "";

      // Extract text content
      let text = "";
      if (msg.message?.conversation) {
        text = msg.message.conversation;
      } else if (msg.message?.extendedTextMessage?.text) {
        text = msg.message.extendedTextMessage.text;
      } else if (msg.message?.imageMessage?.caption) {
        text = msg.message.imageMessage.caption || "[Image received]";
      } else if (msg.message?.videoMessage?.caption) {
        text = msg.message.videoMessage.caption || "[Video received]";
      } else if (msg.message?.documentMessage) {
        text = "[Document received]";
      } else if (msg.message?.audioMessage) {
        text = "[Audio received]";
      } else if (msg.message?.locationMessage) {
        const loc = msg.message.locationMessage;
        text = `[Location: ${loc.degreesLatitude}, ${loc.degreesLongitude}]`;
      } else if (msg.message?.contactMessage) {
        text = "[Contact received]";
      } else if (msg.message?.stickerMessage) {
        text = "[Sticker received]";
      } else {
        // Unknown message type — skip
        continue;
      }

      if (!text || !from) continue;

      logger.info({ from, text: text.substring(0, 80) }, "Incoming message");

      // Mark as read
      try {
        await sock.readMessages([msg.key]);
      } catch (e) {
        // Non-critical
      }

      // Forward to AgentArk webhook in the Meta webhook format (for compatibility)
      const webhookPayload = {
        object: "whatsapp_business_account",
        entry: [
          {
            changes: [
              {
                value: {
                  messages: [
                    {
                      from: from,
                      id: messageId,
                      type: "text",
                      text: { body: text },
                    },
                  ],
                },
              },
            ],
          },
        ],
        _source: "baileys",
      };

      try {
        const headers = { "Content-Type": "application/json" };
        if (AGENTARK_API_KEY) {
          headers["Authorization"] = `Bearer ${AGENTARK_API_KEY}`;
        }

        const resp = await fetch(`${AGENTARK_URL}/webhook/whatsapp`, {
          method: "POST",
          headers,
          body: JSON.stringify(webhookPayload),
        });

        if (!resp.ok) {
          logger.warn({ status: resp.status }, "AgentArk webhook returned error");
        }
      } catch (err) {
        logger.error({ err }, "Failed to forward message to AgentArk");
      }
    }
  });
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

app.listen(BRIDGE_PORT, BRIDGE_HOST, () => {
  logger.info({ port: BRIDGE_PORT, host: BRIDGE_HOST }, "WhatsApp bridge listening");
  startConnection();
});
