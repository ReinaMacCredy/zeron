package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import sh.zeron.android.core.AppModel

@Composable
fun ClaudeConfigSheet(model: AppModel, chatId: String, onDismiss: () -> Unit) {
    LaunchedEffect(Unit) { onDismiss() }
}
