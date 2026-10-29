package sh.zeron.android.ui.session

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import sh.zeron.android.core.SessionExtras

/** "Switch model?" before a change that notes itself in the transcript (iOS `confirm(prompt)`). */
@Composable
fun SwitchPromptDialog(extras: SessionExtras) {
    val pending = extras.pendingSwitch ?: return
    val p = pending.prompt
    AlertDialog(
        onDismissRequest = { extras.resolveSwitch(false) },
        title = { Text(p.title) },
        text = { Text("${p.lead}.\n\n${p.body}") },
        confirmButton = { TextButton(onClick = { extras.resolveSwitch(true) }) { Text(p.confirm) } },
        dismissButton = { TextButton(onClick = { extras.resolveSwitch(false) }) { Text(p.cancel) } },
    )
}
