package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel

@Composable
fun AgentsPill(
    model: AppModel,
    chatId: String,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenTerminal: (terminalId: String?) -> Unit,
) {
}
