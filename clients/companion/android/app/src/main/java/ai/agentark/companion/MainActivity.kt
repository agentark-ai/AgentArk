package ai.agentark.companion

import android.Manifest
import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

class MainActivity : Activity() {
    private val notificationChannelId = "agentark_companion_notifications"
    private lateinit var store: SecureTokenStore
    private var client: CompanionClient? = null
    private lateinit var status: TextView
    private lateinit var wsUrl: EditText
    private lateinit var sessionId: EditText
    private lateinit var code: EditText

    private val bg = Color.rgb(3, 5, 4)
    private val panel = Color.rgb(12, 14, 15)
    private val field = Color.rgb(6, 9, 10)
    private val line = Color.argb(38, 255, 255, 255)
    private val cyan = Color.rgb(124, 231, 255)
    private val textPrimary = Color.rgb(239, 247, 239)
    private val muted = Color.rgb(170, 176, 184)
    private val danger = Color.rgb(255, 155, 155)

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
        window.statusBarColor = bg
        window.navigationBarColor = bg

        val scroll = ScrollView(this).apply {
            setBackgroundColor(bg)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        }
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(22), dp(20), dp(28))
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        }
        scroll.addView(layout)

        layout.addView(brandHeader())

        val connectionPanel = panelLayout().apply {
            addView(label("WebSocket URL"))
            wsUrl = input("ws://localhost:8990/companion/ws").apply {
                setText("ws://10.0.2.2:8990/companion/ws")
            }
            addView(wsUrl)
            addView(label("Pairing session id"))
            sessionId = input("pairing session id")
            addView(sessionId)
            addView(label("Pairing code"))
            code = input("pairing code")
            addView(code)
        }
        layout.addView(connectionPanel)

        val actionsPanel = panelLayout().apply {
            val connect = actionButton("Connect", primary = true).apply {
                setOnClickListener { connectClient() }
            }
            val claim = actionButton("Claim pairing", primary = true).apply {
                setOnClickListener {
                    ensureNotificationPermission()
                    client?.claimPairing(sessionId.text.toString(), code.text.toString())
                }
            }
            val pulse = actionButton("Pulse").apply {
                setOnClickListener { client?.pulse() }
            }
            val clear = actionButton("Clear token", dangerTone = true).apply {
                setOnClickListener {
                    store.clear()
                    status.text = "Stored token cleared"
                }
            }
            listOf(connect, claim, pulse, clear).forEach { addView(it) }
        }
        layout.addView(actionsPanel)

        val statusPanel = panelLayout().apply {
            addView(sectionTitle("Status"))
            status = TextView(this@MainActivity).apply {
                text = "Not connected"
                setTextColor(textPrimary)
                textSize = 15f
            }
            addView(status)
        }
        layout.addView(statusPanel)

        setContentView(scroll)
    }

    override fun onDestroy() {
        client?.disconnect()
        super.onDestroy()
    }

    private fun brandHeader(): LinearLayout {
        val header = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, 0, 0, dp(18))
        }
        header.addView(AgentArkLogoView(this), LinearLayout.LayoutParams(dp(52), dp(52)))
        val copy = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), 0, 0, 0)
        }
        copy.addView(TextView(this).apply {
            text = "AgentArk"
            setTextColor(cyan)
            textSize = 11f
            typeface = Typeface.DEFAULT_BOLD
            letterSpacing = 0.08f
        })
        copy.addView(TextView(this).apply {
            text = "Companion"
            setTextColor(textPrimary)
            textSize = 28f
            typeface = Typeface.DEFAULT_BOLD
        })
        copy.addView(TextView(this).apply {
            text = "Personal AI Agent OS"
            setTextColor(muted)
            textSize = 13f
        })
        header.addView(
            copy,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        )
        return header
    }

    private fun panelLayout(): LinearLayout {
        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(16))
            background = roundedDrawable(panel, line, dp(14))
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            ).apply {
                bottomMargin = dp(12)
            }
        }
    }

    private fun sectionTitle(value: String): TextView {
        return TextView(this).apply {
            text = value
            setTextColor(textPrimary)
            textSize = 18f
            typeface = Typeface.DEFAULT_BOLD
            setPadding(0, 0, 0, dp(8))
        }
    }

    private fun label(value: String): TextView {
        return TextView(this).apply {
            text = value
            setTextColor(muted)
            textSize = 13f
            setPadding(0, dp(8), 0, dp(5))
        }
    }

    private fun input(hintText: String): EditText {
        return EditText(this).apply {
            hint = hintText
            setHintTextColor(Color.argb(150, 213, 216, 223))
            setTextColor(textPrimary)
            textSize = 15f
            singleLine = true
            setPadding(dp(12), 0, dp(12), 0)
            background = roundedDrawable(field, line, dp(10))
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(48)
            )
        }
    }

    private fun actionButton(value: String, primary: Boolean = false, dangerTone: Boolean = false): Button {
        return Button(this).apply {
            text = value
            isAllCaps = false
            setTextColor(if (dangerTone) danger else textPrimary)
            textSize = 15f
            typeface = Typeface.DEFAULT_BOLD
            background = roundedDrawable(
                fill = if (primary) Color.argb(35, 124, 231, 255) else Color.rgb(14, 18, 20),
                stroke = if (primary) Color.argb(150, 124, 231, 255) else if (dangerTone) Color.argb(120, 255, 155, 155) else line,
                radius = dp(10)
            )
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(48)
            ).apply {
                bottomMargin = dp(8)
            }
        }
    }

    private fun roundedDrawable(fill: Int, stroke: Int, radius: Int): GradientDrawable {
        return GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            setColor(fill)
            cornerRadius = radius.toFloat()
            setStroke(dp(1), stroke)
        }
    }

    private fun dp(value: Int): Int {
        return (value * resources.displayMetrics.density).toInt()
    }

    private fun connectClient() {
        ensureNotificationPermission()
        createNotificationChannel()
        client?.disconnect()
        client = CompanionClient(
            wsUrl = wsUrl.text.toString(),
            tokenStore = store,
            capabilities = capabilities,
            onStatus = { runOnUiThread { status.text = it } },
            onLocalNotification = { title, body -> showLocalNotification(title, body) }
        )
        client?.connect()
    }

    private fun ensureNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1001)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            notificationChannelId,
            "AgentArk Companion",
            NotificationManager.IMPORTANCE_DEFAULT
        ).apply {
            description = "Local AgentArk companion notifications"
        }
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.createNotificationChannel(channel)
    }

    private fun showLocalNotification(title: String, body: String): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        createNotificationChannel()
        val intent = Intent(this, MainActivity::class.java)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val pendingIntent = PendingIntent.getActivity(this, 0, intent, flags)
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, notificationChannelId)
        } else {
            Notification.Builder(this)
        }
        val notification = builder
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body))
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .build()
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify((System.currentTimeMillis() % Int.MAX_VALUE).toInt(), notification)
        return true
    }
}

class AgentArkLogoView(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val w = width.toFloat()
        val h = height.toFloat()
        val cx = w / 2f
        val cy = h * 0.55f
        val r = minOf(w, h) * 0.34f

        paint.style = Paint.Style.STROKE
        paint.strokeWidth = r * 0.12f
        paint.strokeCap = Paint.Cap.ROUND
        paint.color = Color.rgb(124, 231, 255)
        canvas.drawLine(cx - r * 0.45f, cy - r * 0.72f, cx - r * 0.78f, cy - r * 1.18f, paint)
        paint.color = Color.rgb(245, 158, 11)
        canvas.drawLine(cx + r * 0.45f, cy - r * 0.72f, cx + r * 0.78f, cy - r * 1.18f, paint)

        paint.style = Paint.Style.FILL
        paint.color = Color.rgb(109, 40, 217)
        canvas.drawOval(RectF(cx - r, cy - r * 0.95f, cx + r, cy + r * 1.08f), paint)
        paint.color = Color.argb(82, 192, 132, 252)
        canvas.drawCircle(cx - r * 0.24f, cy - r * 0.22f, r * 0.82f, paint)

        paint.color = Color.WHITE
        canvas.drawCircle(cx - r * 0.34f, cy - r * 0.16f, r * 0.22f, paint)
        canvas.drawCircle(cx + r * 0.34f, cy - r * 0.16f, r * 0.22f, paint)
        paint.color = Color.rgb(124, 231, 255)
        canvas.drawCircle(cx - r * 0.34f, cy - r * 0.16f, r * 0.10f, paint)
        paint.color = Color.rgb(245, 158, 11)
        canvas.drawCircle(cx + r * 0.34f, cy - r * 0.16f, r * 0.10f, paint)
    }
}
