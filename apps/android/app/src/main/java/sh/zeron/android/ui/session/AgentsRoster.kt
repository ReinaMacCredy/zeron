package sh.zeron.android.ui.session

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.successColor
import uniffi.zeron_core.AgentRunState
import uniffi.zeron_core.BackgroundRosterView
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.ShellRow

@Stable
class RosterState(private val handle: SessionHandle?) {
    var value by mutableStateOf(read())
        private set

    fun reload() {
        value = read()
    }

    private fun read(): BackgroundRosterView? = handle?.let { runCatching { it.backgroundRoster(System.currentTimeMillis()) }.getOrNull() }
}

@Composable
fun rememberRoster(model: AppModel, chatId: String, handle: SessionHandle?, ticking: Boolean): RosterState {
    val state = remember(handle) { RosterState(handle) }
    LaunchedEffect(state) {
        val dirty = Channel<Unit>(Channel.CONFLATED)
        launch { model.sessionEvents.collect { if (it == chatId) dirty.trySend(Unit) } }
        for (unit in dirty) {
            state.reload()
            delay(250)
        }
    }
    if (ticking) {
        LaunchedEffect(state) {
            while (true) {
                state.reload()
                delay(1000)
            }
        }
    }
    return state
}

fun sessionHandle(model: AppModel, chatId: String): SessionHandle? {
    val client = model.client.value ?: return null
    return client.session(chatId) ?: runCatching { client.openSession(chatId) }.getOrNull()
}

fun stopRun(model: AppModel, handle: SessionHandle, scope: CoroutineScope, taskId: String, delegated: Boolean, then: () -> Unit) {
    scope.launch {
        try {
            handle.stopBackgroundRun(taskId, delegated)
            then()
        } catch (e: Exception) {
            model.toast(SessionExtras.failure(e, "Couldn't stop it. Try again."))
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun RunGlyph(state: AgentRunState, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Box(modifier.size(18.dp), contentAlignment = Alignment.Center) {
        when (state) {
            AgentRunState.WORKING -> LoadingIndicator(Modifier.size(20.dp), color = scheme.primary)
            AgentRunState.COMPLETED -> ZIcon(ZIcons.Check, null, Modifier.size(15.dp), tint = successColor())
            AgentRunState.FAILED -> ZIcon(ZIcons.Close, null, Modifier.size(15.dp), tint = scheme.error)
            AgentRunState.STOPPED -> ZIcon(ZIcons.Stop, null, Modifier.size(13.dp), tint = scheme.onSurfaceVariant)
        }
    }
}

@Composable
fun StopButton(label: String, onClick: () -> Unit) {
    FilledTonalButton(
        onClick = onClick,
        contentPadding = PaddingValues(horizontal = 14.dp),
        modifier = Modifier.height(32.dp).semantics { contentDescription = label },
    ) { Text("Stop", style = MaterialTheme.typography.labelLarge) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShellOutputSheet(handle: SessionHandle, shell: ShellRow, onStop: () -> Unit, onDismiss: () -> Unit) {
    var text by remember(shell.taskId) { mutableStateOf("") }
    val scroll = rememberScrollState()
    DisposableEffect(shell.taskId) {
        handle.watchShellOutput(shell.taskId)
        onDispose { handle.watchShellOutput(null) }
    }
    LaunchedEffect(shell.taskId) {
        while (true) {
            val next = runCatching { handle.shellOutput(shell.taskId) }.getOrDefault(emptyList()).joinToString("\n")
            if (next != text) {
                text = next
                scroll.scrollTo(Int.MAX_VALUE)
            }
            delay(1000)
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                shell.label,
                fontFamily = GeistMono,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (shell.canStop) {
                TextButton(onClick = {
                    onStop()
                    onDismiss()
                }) { Text("Stop") }
            }
        }
        Surface(
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHighest,
            modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 24.dp).heightIn(min = 240.dp, max = 520.dp),
        ) {
            Text(
                text,
                fontFamily = GeistMono,
                fontSize = 12.sp,
                lineHeight = 17.sp,
                modifier = Modifier.verticalScroll(scroll).padding(horizontal = 12.dp, vertical = 12.dp),
            )
        }
    }
}
