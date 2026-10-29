package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras

@Composable
fun AgentsPill(
    model: AppModel,
    chatId: String,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenTerminal: (terminalId: String?) -> Unit,
    extras: SessionExtras? = null,
    openCard: Boolean = false,
    onOpenFindings: (taskId: String?) -> Unit = {},
) {
}

@Composable
fun AgentsSheet(
    model: AppModel,
    chatId: String,
    extras: SessionExtras,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenTerminal: (terminalId: String?) -> Unit,
    onOpenFindings: (taskId: String?) -> Unit,
    onDismiss: () -> Unit,
) {
}
