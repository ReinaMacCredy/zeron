package sh.zeron.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import sh.zeron.android.core.SessionExtras
import uniffi.zeron_core.InfoSource

@Composable
fun SessionInfoSheet(
    extras: SessionExtras,
    sources: List<InfoSource>,
    onOpenChanges: (title: String) -> Unit,
    onOpenImage: (reference: String) -> Unit,
    onDismiss: () -> Unit,
) {
    LaunchedEffect(Unit) { onDismiss() }
}
