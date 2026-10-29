package sh.zeron.android.ui.session

import android.view.HapticFeedbackConstants
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import kotlinx.coroutines.delay
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.AgentRunRow
import uniffi.zeron_core.AgentRunState
import uniffi.zeron_core.BackgroundRosterView
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.ShellRow
import uniffi.zeron_core.SubagentState

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
    val handle = extras?.handle ?: return
    var open by remember(chatId) { mutableStateOf(false) }
    var pending by remember(chatId) { mutableStateOf(openCard) }
    LaunchedEffect(openCard) { if (openCard) pending = true }
    val roster = rememberRoster(model, chatId, handle, ticking = open)
    val summary = roster.value?.summary
    LaunchedEffect(summary == null) { if (summary == null) open = false }
    LaunchedEffect(pending, summary != null) {
        if (pending && summary != null) {
            delay(800)
            pending = false
            open = true
        }
    }
    val view = LocalView.current
    val density = LocalDensity.current
    var pillTop by remember { mutableFloatStateOf(0f) }
    val shown = remember { MutableTransitionState(false) }
    shown.targetState = summary != null
    var last by remember { mutableStateOf(summary ?: "") }
    if (summary != null) last = summary
    AnimatedVisibility(shown, enter = fadeIn() + slideInVertically { it / 6 }, exit = fadeOut()) {
        Box(Modifier.onGloballyPositioned { pillTop = it.boundsInWindow().top }) {
            Surface(
                onClick = {
                    view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                    open = !open
                },
                shape = RoundedCornerShape(50),
                color = if (open) MaterialTheme.colorScheme.surfaceContainerHighest else MaterialTheme.colorScheme.surfaceContainerHigh,
                contentColor = if (open) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.height(32.dp).semantics {
                    contentDescription = last
                    stateDescription = if (open) "Open" else ""
                },
            ) {
                Row(Modifier.padding(start = 11.dp, end = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                    RunGlyph(roster.value?.state ?: AgentRunState.WORKING)
                    Spacer(Modifier.width(8.dp))
                    Text(last, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            val r = roster.value
            if (open && r != null) {
                val maxHeight = with(density) { pillTop.toDp() } - 72.dp
                Popup(
                    popupPositionProvider = remember(density) { AboveAnchor(with(density) { 8.dp.roundToPx() }, with(density) { 16.dp.roundToPx() }) },
                    onDismissRequest = { open = false },
                    properties = PopupProperties(focusable = true),
                ) {
                    AgentsCard(
                        model, chatId, handle, r, roster::reload, maxHeight,
                        onClose = { open = false },
                        onOpenChat = onOpenChat,
                        onOpenSubagent = onOpenSubagent,
                        onOpenFindings = onOpenFindings,
                    )
                }
            }
        }
    }
}

private class AboveAnchor(val gap: Int, val margin: Int) : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize) =
        IntOffset(margin, anchorBounds.top - gap - popupContentSize.height)
}

fun openRun(
    model: AppModel,
    chatId: String,
    run: AgentRunRow,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenFindings: (taskId: String?) -> Unit,
    then: (() -> Unit) -> Unit,
) {
    when {
        run.review != null -> then { onOpenFindings(run.taskId) }
        run.delegated -> then { onOpenChat(run.taskId) }
        else -> {
            val sub = model.sessionSubagents(chatId).firstOrNull { it.id == run.taskId } ?: return
            val doc = sub.docId ?: return
            then { onOpenSubagent(doc, sub.title, sub.state == SubagentState.RUNNING) }
        }
    }
}

@Composable
private fun AgentsCard(
    model: AppModel,
    chatId: String,
    handle: SessionHandle,
    roster: BackgroundRosterView,
    reload: () -> Unit,
    maxHeight: Dp,
    onClose: () -> Unit,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenFindings: (taskId: String?) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var shell by remember { mutableStateOf<ShellRow?>(null) }
    val width = LocalConfiguration.current.screenWidthDp.dp - 32.dp
    val visible = remember { MutableTransitionState(false).apply { targetState = true } }
    AnimatedVisibility(visible, enter = fadeIn() + slideInVertically { it / 12 }) {
        Surface(
            shape = RoundedCornerShape(18.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            shadowElevation = 10.dp,
            modifier = Modifier.width(width).heightIn(max = maxHeight.coerceAtLeast(160.dp)),
        ) {
            Column(Modifier.padding(bottom = 6.dp)) {
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        if (roster.agents.isEmpty()) roster.shellsSection else roster.heading,
                        style = MaterialTheme.typography.titleSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = onClose) { ZIcon(ZIcons.Close, "Close", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
                Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
                    roster.agents.forEach { run ->
                        RosterRow(
                            state = run.state,
                            title = run.title,
                            line = listOfNotNull(run.line, run.review).joinToString(" · "),
                            depth = run.depth.toInt(),
                            mono = false,
                            stop = if (run.canStop) ({ stopRun(model, handle, scope, run.taskId, run.delegated, reload) }) else null,
                        ) {
                            openRun(model, chatId, run, onOpenChat, onOpenSubagent, onOpenFindings) { go ->
                                onClose()
                                go()
                            }
                        }
                    }
                    if (roster.agents.isNotEmpty() && roster.shells.isNotEmpty()) {
                        Text(
                            roster.shellsSection,
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 4.dp),
                        )
                    }
                    roster.shells.forEach { s ->
                        RosterRow(
                            state = AgentRunState.WORKING,
                            title = s.label,
                            line = s.line,
                            depth = 0,
                            mono = true,
                            stop = if (s.canStop) ({ stopRun(model, handle, scope, s.taskId, false, reload) }) else null,
                        ) { shell = s }
                    }
                }
            }
        }
    }
    shell?.let { s ->
        ShellOutputSheet(handle, s, onStop = { stopRun(model, handle, scope, s.taskId, false, reload) }) { shell = null }
    }
}

@Composable
private fun RosterRow(state: AgentRunState, title: String, line: String, depth: Int, mono: Boolean, stop: (() -> Unit)?, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 44.dp)
            .padding(start = 14.dp + 16.dp * depth, end = 12.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RunGlyph(state)
        Spacer(Modifier.width(10.dp))
        Text(
            title,
            style = if (mono) MaterialTheme.typography.bodyMedium.copy(fontFamily = GeistMono) else MaterialTheme.typography.bodyLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            line,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.widthIn(max = 190.dp),
        )
        if (stop != null) {
            Spacer(Modifier.width(8.dp))
            StopButton("Stop $title", stop)
        }
    }
}
