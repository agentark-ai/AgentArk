package ai.agentark.companion

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.util.UUID

data class CompanionIdentity(val deviceId: String, val token: String)

class SecureTokenStore(context: Context) {
    private val masterKey = MasterKey.Builder(context)
        .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
        .build()

    private val prefs = EncryptedSharedPreferences.create(
        context,
        "agentark_companion_identity",
        masterKey,
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
    )

    fun load(): CompanionIdentity? {
        val deviceId = prefs.getString("device_id", null) ?: return null
        val token = prefs.getString("token", null) ?: return null
        return CompanionIdentity(deviceId, token)
    }

    fun save(identity: CompanionIdentity) {
        prefs.edit()
            .putString("device_id", identity.deviceId)
            .putString("token", identity.token)
            .apply()
    }

    fun devicePublicKey(): String {
        val existing = prefs.getString("device_public_key", null)
        if (!existing.isNullOrBlank()) return existing
        val generated = "android-${UUID.randomUUID()}"
        prefs.edit().putString("device_public_key", generated).apply()
        return generated
    }

    fun clear() {
        prefs.edit().clear().apply()
    }
}
