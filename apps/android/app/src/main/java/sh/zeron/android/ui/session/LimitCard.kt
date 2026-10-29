package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import sh.zeron.android.core.SessionExtras

@Composable
fun LimitCardSheet(extras: SessionExtras, limitChatId: String, onOpenChat: (String) -> Unit, onDismiss: () -> Unit) {
    LaunchedEffect(Unit) { onDismiss() }
}
