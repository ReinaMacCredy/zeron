package sh.zeron.android.ui.session

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.ui.git.color
import sh.zeron.android.ui.git.rememberDiffPalette
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.LimitCard
import uniffi.zeron_core.LimitChoice

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LimitCardSheet(extras: SessionExtras, limitChatId: String, onOpenChat: (String) -> Unit, onDismiss: () -> Unit) {
    val initial = remember(limitChatId) { runCatching { extras.core.limitCard(limitChatId) }.getOrNull() }
    if (initial == null) {
        LaunchedEffect(limitChatId) {
            onOpenChat(limitChatId)
            onDismiss()
        }
        return
    }
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var card by remember(initial) { mutableStateOf<LimitCard>(initial) }
    var selectedSwitch by remember(initial) { mutableStateOf(initial.preselectSwitch && initial.switch != null) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            now = System.currentTimeMillis()
        }
    }
    fun dismissThen(after: () -> Unit = {}) {
        scope.launch { sheet.hide() }.invokeOnCompletion {
            onDismiss()
            after()
        }
    }
    fun resolve(choice: String) {
        scope.launch {
            try {
                extras.core.resolveLimitOffer(limitChatId, choice)
            } catch (e: Exception) {
                extras.model.toast(SessionExtras.failure(e, "Couldn't reach the host. Try again."))
                return@launch
            }
            if (choice == "hold") card = card.copy(actAtMs = null) else dismissThen()
        }
    }
    fun select(switching: Boolean) {
        if (switching == selectedSwitch) return
        selectedSwitch = switching
        val at = card.actAtMs
        if (switching != card.preselectSwitch && at != null && at > System.currentTimeMillis()) resolve("hold")
    }

    val left = card.actAtMs?.let { ((it - now + 999) / 1000).toInt() } ?: 0
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 24.dp).navigationBarsPadding()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Usage limit", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.weight(1f))
                TextButton(onClick = { dismissThen { onOpenChat(limitChatId) } }) { Text("Open chat") }
            }
            Spacer(Modifier.height(12.dp))
            Text(card.title, style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(4.dp))
            Text(card.resets, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.outline)
            Spacer(Modifier.height(16.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                card.switch?.let { LimitOption(it, chosen = selectedSwitch) { select(true) } }
                LimitOption(card.wait, chosen = !selectedSwitch) { select(false) }
            }
            Spacer(Modifier.height(16.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (left > 0) {
                    Text("${card.countdownVerb} in $left s", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = { resolve("hold") }) { Text("Cancel") }
                }
                Spacer(Modifier.weight(1f))
                Button(
                    onClick = { resolve(if (selectedSwitch) "switch" else "wait") },
                    colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.onSurface, contentColor = MaterialTheme.colorScheme.surface),
                ) { Text((if (selectedSwitch) card.switch?.act else null) ?: card.wait.act) }
            }
        }
    }
}

@Composable
private fun LimitOption(choice: LimitChoice, chosen: Boolean, onSelect: () -> Unit) {
    val warning = rememberDiffPalette().color(ColorRole.WARNING)
    val shape = RoundedCornerShape(16.dp)
    Surface(
        selected = chosen,
        onClick = onSelect,
        shape = shape,
        color = if (chosen) warning.copy(alpha = 0.08f) else MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, if (chosen) warning.copy(alpha = 0.7f) else MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.fillMaxWidth().semantics { role = Role.RadioButton },
    ) {
        Row(Modifier.padding(start = 4.dp, end = 16.dp, top = 4.dp, bottom = 12.dp), verticalAlignment = Alignment.Top) {
            RadioButton(selected = chosen, onClick = null, colors = RadioButtonDefaults.colors(selectedColor = warning, unselectedColor = MaterialTheme.colorScheme.outline), modifier = Modifier.padding(12.dp))
            Column(Modifier.padding(top = 10.dp)) {
                Text(choice.title, style = MaterialTheme.typography.titleSmall)
                Spacer(Modifier.height(2.dp))
                Text(choice.detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
