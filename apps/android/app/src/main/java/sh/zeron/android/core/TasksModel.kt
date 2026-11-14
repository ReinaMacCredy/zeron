package sh.zeron.android.core

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.SandboxLevel
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
    val trees: TaskTrees,
)

private val emptyTasks = TasksPage(emptyList(), emptyList(), emptyList(), emptyList(), emptyList(), 0u)
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
