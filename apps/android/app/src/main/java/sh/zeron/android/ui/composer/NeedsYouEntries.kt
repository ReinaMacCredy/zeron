package sh.zeron.android.ui.composer

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
import sh.zeron.android.ui.frontRows
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot

object NeedsYou {
    fun waiting(ws: WorkspaceSnapshot, excluding: String?): List<SessionRow> =
        frontRows(ws).filter { it.indicator == ChatIndicator.AWAITING_INPUT && !it.archived && it.id != excluding }
}

@Composable
fun rememberWaiting(model: AppModel, excluding: String?): List<SessionRow> {
    val workspace by model.workspace.collectAsState()
    return remember(workspace, excluding) { workspace?.let { NeedsYou.waiting(it, excluding) }.orEmpty() }
}

@Composable
fun rememberOpenInputs(model: AppModel, rows: List<SessionRow>): Map<String, InputRequest> {
    val client by model.client.collectAsState()
    var sessions by remember { mutableIntStateOf(0) }
    LaunchedEffect(model) {
        model.sessionEvents.collectLatest {
            delay(200)
            sessions++
        }
    }
    return remember(client, rows, sessions) {
        val c = client ?: return@remember emptyMap()
        rows.mapNotNull { row ->
            val handle = c.session(row.id) ?: runCatching { c.openSession(row.id) }.getOrNull()
            handle?.let { runCatching { it.composer().openInput }.getOrNull() }?.let { row.id to it }
        }.toMap()
    }
}

@Composable
fun askColor(): Color = if (LocalDarkTheme.current) Color(0xFFF08BC2) else Color(0xFFC2478F)
