package sh.zeron.android.core

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.NeedsYouEntry
import uniffi.zeron_core.NeedsYouKind
import uniffi.zeron_core.SandboxLevel
import uniffi.zeron_core.SupervisorHost
import uniffi.zeron_core.SupervisorPage
import uniffi.zeron_core.SupervisorState
import uniffi.zeron_core.TaskRow
import uniffi.zeron_core.TaskStatus
import uniffi.zeron_core.TaskTrees
import uniffi.zeron_core.TasksPage
import uniffi.zeron_core.relativeTimeLabel
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

data class TasksSnapshot(
    val client: CoreClient?,
    val page: TasksPage,
    val supervisor: SupervisorPage,
    val needsYou: List<NeedsYouEntry>,
    val trees: TaskTrees,
)

private val emptyTasks = TasksPage(emptyList(), emptyList(), emptyList(), emptyList(), emptyList(), emptyList(), 0u)
private val emptySupervisor = SupervisorPage(emptyList(), emptyList(), emptyList())
private val emptyTrees = TaskTrees(emptyList(), emptyList())

@Composable
fun rememberTasks(model: AppModel): TasksSnapshot {
    val client by model.client.collectAsState()
    val ws by model.workspace.collectAsState()
    return remember(client, ws) {
        val c = client
        TasksSnapshot(
            c,
            c?.let { runCatching { it.tasksPage() }.getOrNull() } ?: emptyTasks,
            c?.let { runCatching { it.supervisorPage() }.getOrNull() } ?: emptySupervisor,
            c?.let { runCatching { it.needsYou() }.getOrNull() } ?: emptyList(),
            c?.let { runCatching { it.taskTrees() }.getOrNull() } ?: emptyTrees,
        )
    }
}

object TasksText {
    fun nowMs(): Long = System.currentTimeMillis()

    fun error(e: Throwable): String = when (e) {
        is CoreException.HostUnavailable -> "That Mac is offline."
        is CoreException.Unsupported -> "That Mac's Zeron is too old for this."
        else -> "That didn't go through. Try again."
    }

    fun missionLine(task: TaskRow): String {
        val parts = mutableListOf<String>()
        task.host?.let { parts += it.name }
        parts += task.missionLabel ?: task.word
        if (task.progressTotal > 0u) parts += "${task.progressDone}/${task.progressTotal} steps"
        return parts.joinToString(" · ")
    }

    fun taskLine(task: TaskRow): String {
        val parts = mutableListOf(task.project?.name ?: "No project")
        if (task.status != TaskStatus.DONE) task.dueOn?.let { parts += dueLabel(it) }
        if (task.status == TaskStatus.WORKING || task.status == TaskStatus.NEEDS_YOU) task.host?.let { parts += it.name }
        if (task.status == TaskStatus.DONE) task.completedAtMs?.let { parts += "Done " + relativeTimeLabel(it, nowMs()) }
        return parts.joinToString(" · ")
    }

    fun dueLabel(day: String): String {
        val date = runCatching { LocalDate.parse(day) }.getOrNull() ?: return "due $day"
        val today = LocalDate.now()
        return when (date) {
            today -> "due today"
            today.minusDays(1) -> "due yesterday"
            today.plusDays(1) -> "due tomorrow"
            else -> "due " + date.format(DateTimeFormatter.ofPattern("MMM d"))
        }
    }

    fun supervisorLine(page: SupervisorPage, needs: Int): String {
        val parts = mutableListOf<String>()
        if (needs > 0) parts += "$needs need${if (needs == 1) "s" else ""} you"
        val trees = page.hosts.sumOf { it.trees.toInt() }
        val names = page.hosts.map { it.host.name }
        if (names.isNotEmpty()) {
            val macs = if (names.size == 1) names[0] else names.dropLast(1).joinToString(", ") + " and " + names.last()
            parts += "Watching $trees tree${if (trees == 1) "" else "s"} on $macs"
        }
        page.hosts.mapNotNull { it.lastWakeMs }.maxOrNull()?.let { parts += "last wake " + relativeTimeLabel(it, nowMs()) }
        return if (parts.isEmpty()) "No Mac is running a Supervisor" else parts.joinToString(" · ")
    }

    fun clock(context: Context, ms: Long): String =
        android.text.format.DateFormat.getTimeFormat(context).format(java.util.Date(ms))

    fun hostLine(context: Context, host: SupervisorHost): String {
        if (!host.host.online) return "Offline"
        host.blocked?.let { return "Couldn't start the Supervisor: $it" }
        return when (host.state) {
            SupervisorState.OFF -> "Off"
            SupervisorState.STOPPED -> if (host.waiting > 0u) "Stopped by you · ${host.waiting} wake${if (host.waiting == 1u) "" else "s"} waiting" else "Stopped by you"
            SupervisorState.ASLEEP, SupervisorState.AWAKE -> {
                val parts = mutableListOf(if (host.watchEverything) "Watching everything" else "Watching ${host.trees} tree${if (host.trees == 1u) "" else "s"}")
                parts += "${host.wakesThisHour} of ${host.wakesPerHour} wakes this hour"
                host.nextCheckMs?.let { parts += "next check ${clock(context, it)}" }
                parts.joinToString(" · ")
            }
        }
    }

    fun kindLabel(entry: NeedsYouEntry): String = when (val k = entry.kind) {
        is NeedsYouKind.Input -> if (k.questions.isEmpty()) "Waiting for you" else if (k.approval) "Approval" else "Question"
        is NeedsYouKind.Item -> k.why
        is NeedsYouKind.Plan -> "Plan ready"
    }

    fun rowTitle(entry: NeedsYouEntry): String = if (entry.kind is NeedsYouKind.Input) entry.asker else entry.title

    fun sendLine(entry: NeedsYouEntry): String? = when (val s = entry.send) {
        is uniffi.zeron_core.ItemSend.Sending -> "Sending to ${entry.dest} · → ${s.choice}"
        is uniffi.zeron_core.ItemSend.Sent -> "Sent to ${entry.dest} · → ${s.choice}"
        is uniffi.zeron_core.ItemSend.Failed -> "Could not send: ${s.reason} · ${s.choice} kept"
        null -> null
    }

    fun needsLine(entry: NeedsYouEntry): String {
        sendLine(entry)?.let { return "${entry.host.name} · $it" }
        val parts = mutableListOf(entry.host.name)
        when (val k = entry.kind) {
            is NeedsYouKind.Item -> parts += if (k.raisedByLead) "${k.why} · Raised by Lead" else k.why
            is NeedsYouKind.Input -> parts += kindLabel(entry)
            is NeedsYouKind.Plan -> parts += k.steps.take(2).joinToString(", ") + if (k.steps.size > 2) " +${k.steps.size - 2} more" else ""
        }
        parts += relativeTimeLabel(entry.atMs, nowMs())
        return parts.joinToString(" · ")
    }
}

fun AppModel.lastDraftConfig(): ChatConfig {
    val d = lastDraft
    return ChatConfig(d.harness, d.model, d.effort, emptyMap(), SandboxLevel.WORKSPACE_WRITE)
}

fun AppModel.taskHostId(): String? {
    val hosts = workspace.value?.devices.orEmpty().filter { it.isExecutionHost }
    return lastDraft.hostId?.takeIf { id -> hosts.any { it.id == id && it.online } }
        ?: hosts.firstOrNull { it.online }?.id
        ?: hosts.firstOrNull()?.id
}
