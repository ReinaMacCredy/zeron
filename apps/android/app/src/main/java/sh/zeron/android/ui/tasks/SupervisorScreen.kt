package sh.zeron.android.ui.tasks

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.TasksText
import sh.zeron.android.core.lastDraftConfig
import sh.zeron.android.core.rememberTasks
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.HostRef
import uniffi.zeron_core.NeedsYouEntry
import uniffi.zeron_core.NeedsYouKind
import uniffi.zeron_core.SupervisorHost
import uniffi.zeron_core.SupervisorSettings
import uniffi.zeron_core.SupervisorState

private enum class Filter(val label: String) {
    All("All"), Questions("Questions"), Decisions("Decisions"), Plans("Plans");

    fun matches(e: NeedsYouEntry) = when (this) {
        All -> true
        Questions -> e.kind is NeedsYouKind.Input
        Decisions -> e.kind is NeedsYouKind.Item
        Plans -> e.kind is NeedsYouKind.Plan
    }
}

private sealed interface SvSub {
    data class Mission(val taskId: String) : SvSub
    data class Authority(val host: HostRef) : SvSub
}

@Composable
fun SupervisorPage(model: AppModel, onBack: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: ((String) -> Unit)?) {
    val snap = rememberTasks(model)
    val context = LocalContext.current
    var filter by remember { mutableStateOf(Filter.All) }
    var filterMenu by remember { mutableStateOf(false) }
    var hostSheet by remember { mutableStateOf<SupervisorHost?>(null) }
    var stopping by remember { mutableStateOf<SupervisorHost?>(null) }
    var sub by remember { mutableStateOf<SvSub?>(null) }
    val scope = rememberCoroutineScope()
    fun run(action: suspend (CoreClient) -> Unit) {
        val c = snap.client ?: return
        scope.launch { runCatching { action(c) }.onFailure { model.toast(TasksText.error(it)) } }
    }
    when (val s = sub) {
        is SvSub.Mission -> {
            BackHandler { sub = null }
            MissionControl(model, s.taskId, onBack = { sub = null }, onOpenChat = onOpenChat, onOpenRoute = { r -> onOpenRoute?.invoke(r) })
            return
        }
        is SvSub.Authority -> {
            BackHandler { sub = null }
            AuthorityPage(model, s.host, onBack = { sub = null })
            return
        }
        null -> Unit
    }
    val page = snap.supervisor
    val supervised = snap.needsYou.filter { it.supervised }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar("Supervisor", onBack) {
            Box {
                TextButton(onClick = { filterMenu = true }) { Text(if (filter == Filter.All) "Filter" else filter.label) }
                DropdownMenu(filterMenu, onDismissRequest = { filterMenu = false }) {
                    for (f in Filter.entries) {
                        DropdownMenuItem(
                            text = { Text("${f.label} · ${supervised.count(f::matches)}") },
                            trailingIcon = if (f == filter) ({ sh.zeron.android.design.ZIcon(ZIcons.Check, null) }) else null,
                            onClick = {
                                filter = f
                                filterMenu = false
                            },
                        )
                    }
                }
            }
        }
        val shown = supervised.filter(filter::matches)
        val groups = shown.groupBy { "${it.host.deviceId}|${it.groupKey}" }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
            if (page.hosts.isNotEmpty()) {
                group("hosts", "Status · ${page.hosts.size} Mac${if (page.hosts.size == 1) "" else "s"}") {
                    page.hosts.forEachIndexed { i, host ->
                        if (i > 0) RowDivider()
                        ListRow(host.host.name, TasksText.hostLine(context, host), icon = ZIcons.Laptop, chevron = true, onClick = { hostSheet = host })
                    }
                }
            }
            if (shown.isEmpty()) {
                group("needs-empty", "Needs you") {
                    ListRow(if (filter == Filter.All) "Nothing needs you" else "No ${filter.label.lowercase()}", tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            val titles = mutableSetOf<String>()
            for ((key, rows) in groups) {
                val first = rows.first()
                val base = "${first.groupTitle} · ${rows.size} to answer"
                val title = if (titles.add(base)) base else "$base · ${first.host.name}"
                group("needs:$key", title) {
                    rows.forEachIndexed { i, entry ->
                        if (i > 0) RowDivider()
                        ListRow(
                            TasksText.rowTitle(entry),
                            TasksText.needsLine(entry),
                            glyph = Glyph.Dot(if (entry.kind is NeedsYouKind.Input) Tones.input else Tones.warning),
                            chevron = true,
                            onClick = { onOpenRoute?.invoke("needs-you") ?: onOpenChat(entry.chatId) },
                        )
                    }
                }
            }
            if (page.doneToday.isNotEmpty()) {
                group("done", "Done today · ${page.doneToday.size}") {
                    page.doneToday.forEachIndexed { i, task ->
                        if (i > 0) RowDivider()
                        val parts = listOfNotNull(
                            task.host?.name,
                            if (task.progressTotal > 0u) "Done · ${task.progressDone} of ${task.progressTotal}" else "Done",
                            task.completedAtMs?.let { TasksText.clock(context, it) },
                        )
                        ListRow(task.title, parts.joinToString(" · "), glyph = Glyph.Check(Tones.done), chevron = true, onClick = {
                            onOpenRoute?.invoke("tasks:mission:${task.id}") ?: run { sub = SvSub.Mission(task.id) }
                        })
                    }
                }
            }
            if (page.wakes.isNotEmpty()) {
                group("wakes", "Wake log · ${page.wakes.size}") {
                    page.wakes.forEachIndexed { i, wake ->
                        if (i > 0) RowDivider()
                        val events = if (wake.events.isEmpty()) "" else " · ${wake.events.size} event${if (wake.events.size == 1) "" else "s"} · " + wake.events.joinToString(", ")
                        ListRow(
                            TasksText.clock(context, wake.atMs) + events,
                            "${wake.host.name} · ${wake.delivery}",
                            tint = if (wake.woke) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
    hostSheet?.let { host ->
        val device = host.host.deviceId
        val choices = buildList {
            when {
                host.blocked != null || host.state == SupervisorState.OFF -> add((if (host.blocked == null) "Start" else "Try again") to false)
                host.state == SupervisorState.STOPPED -> add("Resume" to false)
                else -> add("Stop" to true)
            }
            if (host.chatId != null) add("Open conversation" to false)
            add((if (host.watchEverything) "Stop watching everything" else "Watch everything") to false)
            add("Authority…" to false)
        }
        ChoiceDialog("Supervisor on ${host.host.name}", TasksText.hostLine(context, host), choices, onDismiss = { hostSheet = null }) { choice ->
            hostSheet = null
            when (choice) {
                "Start", "Try again" -> run { it.ensureSupervisor(device, model.lastDraftConfig()) }
                "Resume" -> run { it.resumeSupervisor(device) }
                "Stop" -> stopping = host
                "Open conversation" -> host.chatId?.let(onOpenChat)
                "Authority…" -> sub = SvSub.Authority(host.host)
                else -> run { it.setWatchEverything(device, !host.watchEverything) }
            }
        }
    }
    stopping?.let { host ->
        ChoiceDialog("Stop the Supervisor on ${host.host.name}?", "Wakes wait until you resume it.", listOf("Stop" to true), onDismiss = { stopping = null }) {
            stopping = null
            run { it.stopSupervisor(host.host.deviceId) }
        }
    }
}

@Composable
fun AuthorityPage(model: AppModel, host: HostRef, onBack: () -> Unit) {
    val client by model.client.collectAsState()
    var settings by remember { mutableStateOf<SupervisorSettings?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(client) {
        val c = client ?: return@LaunchedEffect
        runCatching { c.supervisorSettings(host.deviceId) }
            .onSuccess { settings = it; failure = null }
            .onFailure { failure = TasksText.error(it) }
    }
    fun toggle(after: SupervisorSettings) {
        val before = settings ?: return
        val c = client ?: return
        settings = after
        scope.launch {
            runCatching { c.setSupervisorSettings(host.deviceId, before, after) }
                .onSuccess { settings = it }
                .onFailure {
                    settings = before
                    model.toast(TasksText.error(it))
                }
        }
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar("Supervisor on ${host.name}", onBack)
        val s = settings
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
            if (s == null) {
                group("loading", null) { ListRow(failure ?: "Loading…", tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                return@LazyColumn
            }
            @Composable fun sw(on: Boolean, change: (Boolean) -> Unit) = Switch(on, change)
            group("full", "Full authority") {
                ListRow(
                    "Decide for me",
                    "Answers the Lead's Needs you decisions and approves plans. Each choice shows in the feed with why, and you can change it.",
                    trailing = { sw(s.decideForMe) { toggle(s.copy(decideForMe = it)) } },
                )
                RowDivider()
                ListRow("Change a mission's goal or spend", "Lets it decide those too.", trailing = { sw(s.goalAndSpend) { toggle(s.copy(goalAndSpend = it)) } })
            }
            item {
                Text(
                    "Each Mac runs its own Supervisor; these switches apply to this one.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 32.dp, end = 32.dp, bottom = 18.dp),
                )
            }
            group("also", "Also allowed") {
                ListRow("Message a Lead or Peer directly", trailing = { sw(s.messageThreads) { toggle(s.copy(messageThreads = it)) } })
                RowDivider()
                ListRow("Re-brief a step, through its Lead", trailing = { sw(s.rebrief) { toggle(s.copy(rebrief = it)) } })
                RowDivider()
                ListRow("Pause a step that blocks a shared resource", trailing = { sw(s.pauseSteps) { toggle(s.copy(pauseSteps = it)) } })
                RowDivider()
                ListRow("Start a new mission", trailing = { sw(s.startMissions) { toggle(s.copy(startMissions = it)) } })
            }
            group("heartbeat", "Check in on its own") {
                ListRow(if (s.heartbeatMinutes == 0u) "Off" else "Every ${s.heartbeatMinutes} min", tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
