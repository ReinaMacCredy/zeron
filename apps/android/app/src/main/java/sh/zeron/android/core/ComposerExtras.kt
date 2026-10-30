package sh.zeron.android.core

import android.content.Context
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.InvocationCatalog
import java.util.concurrent.ConcurrentHashMap

object ComposerExtras {
    private val catalogs = ConcurrentHashMap<String, InvocationCatalog>()

    suspend fun invocationCatalog(client: CoreClient, deviceId: String, harness: String, chatId: String?, spaceId: String?, skill: Boolean): InvocationCatalog {
        val key = "$deviceId|${spaceId.orEmpty()}|$harness|$skill"
        catalogs[key]?.let { return it }
        val found = client.invocationCatalog(deviceId, harness, chatId, spaceId, skill)
        if (found.warning == null) catalogs[key] = found
        return found
    }

    fun skillNames(deviceId: String, harness: String): List<String> =
        catalogs.filterKeys { it.startsWith("$deviceId|") && it.endsWith("|$harness|true") }.values.flatMap { c -> c.entries.map { it.name } }

    fun missionLead(context: Context): Boolean =
        context.getSharedPreferences("settings", 0).getString("chatMission", "lead") != "newMission"
}
