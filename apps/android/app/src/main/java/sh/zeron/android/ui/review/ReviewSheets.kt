package sh.zeron.android.ui.review

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import sh.zeron.android.core.SessionExtras

@Composable
fun ReviewOptionsSheet(extras: SessionExtras, onDismiss: () -> Unit) {
    LaunchedEffect(Unit) { onDismiss() }
}

@Composable
fun ReviewFindingsSheet(extras: SessionExtras, taskId: String?, onDismiss: () -> Unit) {
    LaunchedEffect(Unit) { onDismiss() }
}

@Composable
fun ChangesSheet(extras: SessionExtras, title: String, onDismiss: () -> Unit) {
    LaunchedEffect(Unit) { onDismiss() }
}
