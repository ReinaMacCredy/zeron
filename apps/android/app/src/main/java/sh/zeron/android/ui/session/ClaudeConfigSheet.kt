package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras
import uniffi.zeron_core.ConfigTab

@Composable
fun ClaudeConfigSheet(
    model: AppModel,
    chatId: String,
    extras: SessionExtras,
    tab: ConfigTab = ConfigTab.CONFIG,
    query: String = "",
    onOpenUsage: () -> Unit = {},
    onDismiss: () -> Unit,
) {
    LaunchedEffect(Unit) { onDismiss() }
}
