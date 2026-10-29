package sh.zeron.android.core

import android.content.Context
import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.SidebarOrganization
import uniffi.zeron_core.SidebarSort
import uniffi.zeron_core.WhatsNewCard
import uniffi.zeron_core.svgIconPng
import uniffi.zeron_core.whatsNewCard

class HomeModel private constructor(context: Context, private val model: AppModel) {
    private val prefs = context.getSharedPreferences("home", 0)
    private val scope = MainScope()

    private val _organization = MutableStateFlow(
        prefs.getString("organization", null)?.let { runCatching { SidebarOrganization.valueOf(it) }.getOrNull() }
            ?: if (model.groupByProject.value) SidebarOrganization.BY_PROJECT else SidebarOrganization.INBOX,
    )
    val organization: StateFlow<SidebarOrganization> = _organization.asStateFlow()

    private val _sort = MutableStateFlow(
        prefs.getString("sort", null)?.let { runCatching { SidebarSort.valueOf(it) }.getOrNull() } ?: SidebarSort.LAST_UPDATED,
    )
    val sort: StateFlow<SidebarSort> = _sort.asStateFlow()

    private val _activeFirst = MutableStateFlow(prefs.getBoolean("activeFirst", false))
    val activeFirst: StateFlow<Boolean> = _activeFirst.asStateFlow()

    private val _closedLeads = MutableStateFlow(prefs.getStringSet("closedLeads", emptySet())!!.toSet())
    val closedLeads: StateFlow<Set<String>> = _closedLeads.asStateFlow()

    private val _settledOpen = MutableStateFlow(prefs.getBoolean("settledOpen", false))
    val settledOpen: StateFlow<Boolean> = _settledOpen.asStateFlow()

    private val _icons = MutableStateFlow<Map<String, ImageBitmap>>(emptyMap())
    val icons: StateFlow<Map<String, ImageBitmap>> = _icons.asStateFlow()
    private val iconDevices = HashSet<String>()

    init {
        scope.launch {
            model.groupByProject.collect { on ->
                val org = _organization.value
                if (on && org != SidebarOrganization.BY_PROJECT) setOrganization(SidebarOrganization.BY_PROJECT)
                if (!on && org == SidebarOrganization.BY_PROJECT) setOrganization(SidebarOrganization.INBOX)
            }
        }
    }

    fun setOrganization(value: SidebarOrganization) {
        if (value == _organization.value) return
        _organization.value = value
        prefs.edit().putString("organization", value.name).apply()
        if (model.groupByProject.value != (value == SidebarOrganization.BY_PROJECT)) model.setGroupByProject(value == SidebarOrganization.BY_PROJECT)
        apply(model.client.value)
    }

    fun setSort(value: SidebarSort) {
        _sort.value = value
        prefs.edit().putString("sort", value.name).apply()
        apply(model.client.value)
    }

    fun setActiveFirst(on: Boolean) {
        _activeFirst.value = on
        prefs.edit().putBoolean("activeFirst", on).apply()
        apply(model.client.value)
    }

    fun apply(client: CoreClient?) {
        client ?: return
        client.setSidebarView(_organization.value, _sort.value, _activeFirst.value)
        model.refreshWorkspace()
    }

    fun toggleLead(id: String) {
        _closedLeads.value = if (id in _closedLeads.value) _closedLeads.value - id else _closedLeads.value + id
        prefs.edit().putStringSet("closedLeads", _closedLeads.value).apply()
    }

    fun setSettledOpen(open: Boolean) {
        _settledOpen.value = open
        prefs.edit().putBoolean("settledOpen", open).apply()
    }

    fun refreshToday() {
        val client = model.client.value ?: return
        scope.launch { client.refreshToday() }
    }

    suspend fun addTodayTask(title: String) {
        model.client.value?.addTodayTask(title)
    }

    suspend fun setTaskCompleted(id: String, done: Boolean) {
        model.client.value?.setTaskCompleted(id, done)
    }

    fun whatsNew(): WhatsNewCard? {
        val check = whatsNewCard(prefs.getString("whatsNewLastSeen", null))
        if (check.advance) prefs.edit().putString("whatsNewLastSeen", check.version).apply()
        return check.takeIf { it.entry != null }
    }

    fun dismissWhatsNew(version: String) {
        prefs.edit().putString("whatsNewLastSeen", version).apply()
    }

    fun icon(deviceId: String, instanceId: String?): ImageBitmap? {
        if (instanceId == null || deviceId.isEmpty()) return null
        _icons.value["$deviceId/$instanceId"]?.let { return it }
        val client = model.client.value ?: return null
        if (!iconDevices.add(deviceId)) return null
        scope.launch {
            val loaded = HashMap<String, ImageBitmap>()
            for (row in runCatching { client.listProviderInstances(deviceId) }.getOrDefault(emptyList())) {
                val path = row.iconPath ?: continue
                val svg = runCatching { client.readAttachment(deviceId, path) }.getOrNull() ?: continue
                val png = svgIconPng(svg, 48u) ?: continue
                val bitmap = BitmapFactory.decodeByteArray(png, 0, png.size) ?: continue
                loaded["$deviceId/${row.id}"] = bitmap.asImageBitmap()
            }
            if (loaded.isNotEmpty()) _icons.value = _icons.value + loaded
        }
        return null
    }

    companion object {
        private var shared: Pair<AppModel, HomeModel>? = null

        fun of(context: Context, model: AppModel): HomeModel =
            shared?.takeIf { it.first === model }?.second
                ?: HomeModel(context.applicationContext, model).also { shared = model to it }
    }
}
