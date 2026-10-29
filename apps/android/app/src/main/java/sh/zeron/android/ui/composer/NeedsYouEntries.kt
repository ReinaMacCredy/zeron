package sh.zeron.android.ui.composer

import androidx.annotation.DrawableRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.ItemSend
import uniffi.zeron_core.NeedsYouEntry
import uniffi.zeron_core.NeedsYouKind
import uniffi.zeron_core.relativeTimeLabel

enum class NeedsYouFilter(val label: String) { All("All"), Questions("Questions"), Decisions("Decisions"), Plans("Plans") }

val NeedsYouEntry.input: NeedsYouKind.Input? get() = kind as? NeedsYouKind.Input

val NeedsYouEntry.itemId: String? get() = (kind as? NeedsYouKind.Item)?.itemId

val NeedsYouEntry.isThread: Boolean get() = input != null && taskId == null && groupKey.isNotEmpty()

val NeedsYouEntry.filter: NeedsYouFilter
    get() = when (kind) {
        is NeedsYouKind.Input -> NeedsYouFilter.Questions
        is NeedsYouKind.Item -> NeedsYouFilter.Decisions
        is NeedsYouKind.Plan -> NeedsYouFilter.Plans
    }

@get:DrawableRes
val NeedsYouEntry.icon: Int
    get() = when (val k = kind) {
        is NeedsYouKind.Input -> if (k.approval) ZIcons.Warning else ZIcons.Info
        is NeedsYouKind.Item -> ZIcons.Info
        is NeedsYouKind.Plan -> ZIcons.TabTasks
    }

val NeedsYouEntry.kindLabel: String
    get() = when (val k = kind) {
        is NeedsYouKind.Input -> if (k.questions.isEmpty()) "Waiting for you" else if (k.approval) "Approval" else "Question"
        is NeedsYouKind.Item -> k.why
        is NeedsYouKind.Plan -> "Plan ready"
    }

val NeedsYouEntry.actionLabel: String
    get() = when (val k = kind) {
        is NeedsYouKind.Input -> if (k.questions.isEmpty()) "Open" else if (k.approval) "Approve" else "Answer"
        is NeedsYouKind.Item -> "Answer"
        is NeedsYouKind.Plan -> "Approve"
    }

val NeedsYouEntry.rowTitle: String get() = if (input == null) title else asker

val NeedsYouEntry.failure: String? get() = (send as? ItemSend.Failed)?.reason

val NeedsYouEntry.locked: Boolean get() = send is ItemSend.Sending || send is ItemSend.Sent

val NeedsYouEntry.sendLine: String?
    get() = when (val s = send) {
        is ItemSend.Sending -> "Sending to $dest · → ${s.choice}"
        is ItemSend.Sent -> "Sent to $dest · → ${s.choice}"
        is ItemSend.Failed -> "Could not send: ${s.reason} · ${s.choice} kept"
        null -> null
    }

fun NeedsYouEntry.meta(model: AppModel, nowMs: Long = System.currentTimeMillis()): String {
    val parts = mutableListOf(kindLabel)
    when (val k = kind) {
        is NeedsYouKind.Input -> if (isThread) {
            model.row(chatId)?.project?.name?.let { parts.add(it) }
            parts.add(relativeTimeLabel(atMs, nowMs))
        } else if (k.questions.isNotEmpty()) {
            parts.add(title)
        }
        is NeedsYouKind.Item -> parts.add(if (k.raisedByLead) "Raised by Lead" else "Raised by Supervisor")
        is NeedsYouKind.Plan -> return asker
    }
    return parts.joinToString(" · ")
}

object NeedsYou {
    fun all(entries: List<NeedsYouEntry>, excluding: String?) = entries.filter { it.input == null || it.chatId != excluding }

    fun waiting(entries: List<NeedsYouEntry>, excluding: String?) = entries.filter { it.input != null && it.chatId != excluding }
}

@Composable
fun rememberNeedsYou(model: AppModel, tick: Int = 0): List<NeedsYouEntry> {
    val client by model.client.collectAsState()
    val workspace by model.workspace.collectAsState()
    var sessions by remember { mutableIntStateOf(0) }
    LaunchedEffect(model) {
        model.sessionEvents.collectLatest {
            delay(200)
            sessions++
        }
    }
    return remember(client, workspace, sessions, tick) { client?.needsYou().orEmpty() }
}

@Composable
fun askColor(): Color = if (LocalDarkTheme.current) Color(0xFFF08BC2) else Color(0xFFC2478F)

@Composable
fun warningColor(): Color = if (LocalDarkTheme.current) Color(0xFFFACC15) else Color(0xFFA16207)
