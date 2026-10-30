package sh.zeron.android.core

import android.content.Context
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.PrDevice
import uniffi.zeron_core.PrRepository
import uniffi.zeron_core.WorkspaceSnapshot

class GitModel(context: Context) {
    private val prefs = context.getSharedPreferences("git", Context.MODE_PRIVATE)

    var device: String?
        get() = prefs.getString("pr.device", null)
        set(value) = prefs.edit().putString("pr.device", value).apply()

    var repository: String?
        get() = prefs.getString("pr.repository", null)
        set(value) = prefs.edit().putString("pr.repository", value).apply()

    var stars: List<String>
        get() = prefs.getString("pr.stars", null)?.split('\n')?.filter { it.isNotEmpty() }.orEmpty()
        set(value) = prefs.edit().putString("pr.stars", value.joinToString("\n")).apply()

    fun toggleStar(key: String): List<String> {
        val next = stars.let { if (key in it) it - key else it + key }
        stars = next
        return next
    }

    fun pickDevice(devices: List<PrDevice>): String? {
        val saved = device
        return (devices.firstOrNull { it.id == saved && it.online } ?: devices.firstOrNull { it.online } ?: devices.firstOrNull())?.id
    }

    suspend fun pickBoardDevice(client: CoreClient, devices: List<PrDevice>): Pair<String, List<PrRepository>>? {
        for (d in devices.filter { it.online }.sortedBy { it.id != device }) {
            val repositories = client.pullRequestRepositories(d.id)
            if (repositories.isNotEmpty()) return d.id to repositories
        }
        return null
    }

    fun pickRepository(repositories: List<PrRepository>): String? =
        (repositories.firstOrNull { it.repository == repository } ?: repositories.firstOrNull())?.repository

    fun projectFor(deviceId: String?, repository: String?, repositories: List<PrRepository>, workspace: WorkspaceSnapshot?): String? =
        repositories.firstOrNull { it.repository == repository }?.projectId
            ?: workspace?.projects?.firstOrNull { it.deviceId == deviceId && it.gitDetected }?.id

    suspend fun resolveGitTarget(client: CoreClient, workspace: WorkspaceSnapshot?): Pair<String, String?>? {
        val deviceId = pickDevice(client.pullRequestDevices())
            ?: workspace?.projects?.firstOrNull { it.gitDetected }?.deviceId
            ?: return null
        val repositories = client.pullRequestRepositories(deviceId)
        return deviceId to projectFor(deviceId, pickRepository(repositories), repositories, workspace)
    }
}
