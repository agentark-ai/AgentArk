package ai.agentark.companion

import android.app.Activity
import android.os.Bundle
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

class MainActivity : Activity() {
    private lateinit var store: SecureTokenStore
    private var client: CompanionClient? = null
    private lateinit var status: TextView
    private lateinit var wsUrl: EditText
    private lateinit var sessionId: EditText
    private lateinit var code: EditText

    private val capabilities = setOf(
        "approval_prompt",
        "notifications",
        "sms",
        "whatsapp_handoff",
        "camera",
        "photos",
        "location"
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = SecureTokenStore(this)

        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(32, 32, 32, 32)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        }
        wsUrl = EditText(this).apply {
            hint = "ws://localhost:8990/companion/ws"
            setText("ws://10.0.2.2:8990/companion/ws")
        }
        sessionId = EditText(this).apply { hint = "pairing session id" }
        code = EditText(this).apply { hint = "pairing code" }
        status = TextView(this).apply { text = "Not connected" }

        val connect = Button(this).apply {
            text = "Connect"
            setOnClickListener { connectClient() }
        }
        val claim = Button(this).apply {
            text = "Claim pairing"
            setOnClickListener { client?.claimPairing(sessionId.text.toString(), code.text.toString()) }
        }
        val pulse = Button(this).apply {
            text = "Pulse"
            setOnClickListener { client?.pulse() }
        }
        val clear = Button(this).apply {
            text = "Clear token"
            setOnClickListener {
                store.clear()
                status.text = "Stored token cleared"
            }
        }

        listOf(wsUrl, sessionId, code, connect, claim, pulse, clear, status).forEach(layout::addView)
        setContentView(layout)
    }

    override fun onDestroy() {
        client?.disconnect()
        super.onDestroy()
    }

    private fun connectClient() {
        client?.disconnect()
        client = CompanionClient(
            wsUrl = wsUrl.text.toString(),
            tokenStore = store,
            capabilities = capabilities,
            onStatus = { runOnUiThread { status.text = it } }
        )
        client?.connect()
    }
}
