package ai.agentark.companion

import android.os.Handler
import android.os.Looper
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONArray
import org.json.JSONObject

class CompanionClient(
    private val wsUrl: String,
    private val tokenStore: SecureTokenStore,
    private val capabilities: Set<String>,
    private val onStatus: (String) -> Unit
) {
    private val client = OkHttpClient()
    private val mainHandler = Handler(Looper.getMainLooper())
    private var webSocket: WebSocket? = null
    private var pendingPairingSessionId: String? = null
    private var pendingPairingCode: String? = null

    fun connect() {
        val requestBuilder = Request.Builder().url(wsUrl)
        tokenStore.load()?.let { identity ->
            requestBuilder
                .header("Authorization", "Bearer ${identity.token}")
                .header("X-AgentArk-Companion-Device", identity.deviceId)
        }
        val request = requestBuilder.build()
        webSocket = client.newWebSocket(request, listener)
    }

    fun disconnect() {
        mainHandler.removeCallbacksAndMessages(null)
        webSocket?.close(1000, "closed")
        webSocket = null
    }

    fun claimPairing(sessionId: String, code: String) {
        pendingPairingSessionId = sessionId
        pendingPairingCode = code
        sendPairingClaim(sessionId, code)
    }

    private fun sendPairingClaim(sessionId: String, code: String) {
        send(
            JSONObject()
                .put("type", "pairing_claim")
                .put("session_id", sessionId)
                .put("code", code)
                .put("device_public_key", tokenStore.devicePublicKey())
                .put("metadata", JSONObject().put("platform", "android").put("client", "AgentArk Android"))
        )
    }

    fun authenticate() {
        val identity = tokenStore.load()
        if (identity == null) {
            onStatus("No stored companion token")
            return
        }
        onStatus("Stored token will be sent in WebSocket headers on reconnect")
    }

    fun pulse() {
        send(
            JSONObject()
                .put("type", "pulse")
                .put("state", "online")
                .put("capabilities", JSONArray(capabilities.toList().sorted()))
                .put("metadata", JSONObject().put("version", "0.1.0"))
        )
    }

    private fun send(payload: JSONObject) {
        webSocket?.send(payload.toString()) ?: onStatus("WebSocket is not connected")
    }

    private val listener = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            onStatus("Connected")
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            val message = JSONObject(text)
            when (message.optString("type")) {
                "hello" -> onStatus("Protocol ready")
                "auth_ok" -> {
                    onStatus("Authenticated")
                    pulse()
                }
                "auth_error" -> onStatus(message.optString("error", "Auth failed"))
                "pulse_ok" -> onStatus("Pulse accepted")
                "pairing_claim_result" -> handlePairingResult(message)
                "command_dispatch" -> handleCommand(message.optJSONObject("command") ?: JSONObject())
                "command_result_ok" -> pulse()
                "error" -> onStatus(message.optString("error", "Companion error"))
            }
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            onStatus("WebSocket failed: ${t.message ?: t.javaClass.simpleName}")
        }
    }

    private fun handlePairingResult(message: JSONObject) {
        val result = message.optJSONObject("result") ?: return
        val token = result.optString("device_token", "")
        val deviceId = result.optJSONObject("device")?.optString("id", "") ?: ""
        if (token.isNotBlank() && deviceId.isNotBlank()) {
            mainHandler.removeCallbacksAndMessages(null)
            pendingPairingSessionId = null
            pendingPairingCode = null
            tokenStore.save(CompanionIdentity(deviceId, token))
            onStatus("Pairing completed and token stored")
            pulse()
        } else {
            val status = result.optString("status", "")
            onStatus(result.optString("message", "Pairing claim submitted"))
            if (status == "claimed" || status == "approved") {
                schedulePairingRetry()
            }
        }
    }

    private fun schedulePairingRetry() {
        val sessionId = pendingPairingSessionId ?: return
        val code = pendingPairingCode ?: return
        mainHandler.removeCallbacksAndMessages(null)
        mainHandler.postDelayed({ sendPairingClaim(sessionId, code) }, 3000)
    }

    private fun handleCommand(command: JSONObject) {
        val commandId = command.optString("id")
        val capability = command.optString("capability")
        if (!capabilities.contains(capability)) {
            commandResult(commandId, false, null, "Unsupported capability")
            return
        }

        when (capability) {
            "approval_prompt", "notifications" -> commandResult(
                commandId,
                true,
                "Received ${command.optString("action")}",
                null
            )
            else -> commandResult(commandId, false, null, "No Android adapter is installed for $capability")
        }
    }

    private fun commandResult(commandId: String, success: Boolean, preview: String?, error: String?) {
        val payload = JSONObject()
            .put("type", "command_result")
            .put("command_id", commandId)
            .put("success", success)
        if (preview != null) payload.put("result_preview", preview)
        if (error != null) payload.put("error", error)
        send(payload)
    }
}
