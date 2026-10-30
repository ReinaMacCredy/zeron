package sh.zeron.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.TonalCircleButton
import sh.zeron.android.ui.successColor
import uniffi.zeron_core.AgentTerminalScreen
import uniffi.zeron_core.AgentTerminalTab
import uniffi.zeron_core.RunState
import uniffi.zeron_core.SessionHandle

@Composable
fun AgentTerminalPane(handle: SessionHandle, terminalId: String?, onBack: () -> Unit, onFailure: (String) -> Unit) {
    val dark = LocalDarkTheme.current
    var selected by remember { mutableStateOf(terminalId) }
    var tabs by remember { mutableStateOf<List<AgentTerminalTab>?>(null) }
    var screen by remember { mutableStateOf<AgentTerminalScreen?>(null) }
    val scroll = rememberScrollState()
    val host = remember(handle) { handle.composer().host.name }

    LaunchedEffect(selected, dark) {
        while (true) {
            try {
                val all = handle.listTerminals()
                tabs = all
                if (selected?.let { id -> all.none { it.id == id } } != false) selected = all.firstOrNull()?.id
                val id = selected ?: return@LaunchedEffect
                val next = handle.readTerminal(id, dark)
                if (id == selected) screen = next
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                onFailure("Couldn't read the terminal. Try again.")
                return@LaunchedEffect
            }
            delay(1000)
        }
    }
    LaunchedEffect(screen) {
        if (scroll.maxValue - scroll.value < 200) scroll.scrollTo(scroll.maxValue)
    }

    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp).testTag("agent-terminal")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, container = MaterialTheme.colorScheme.surfaceContainerHighest)
            Spacer(Modifier.width(12.dp))
            Text("Terminal", style = MaterialTheme.typography.titleLarge)
        }
        Spacer(Modifier.height(10.dp))
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            tabs.orEmpty().forEach { tab ->
                FilterChip(
                    selected = tab.id == selected,
                    onClick = { selected = tab.id },
                    label = { Text(if (tab.state == RunState.RUNNING) tab.title else "${tab.title} · ${tab.status}") },
                    leadingIcon = { Box(Modifier.size(6.dp).clip(CircleShape).background(stateColor(tab.state))) },
                    modifier = Modifier.testTag("terminal-tab-${tab.id}"),
                )
            }
        }
        Spacer(Modifier.height(10.dp))
        val shown = screen
        if (tabs?.isEmpty() == true) {
            Text(
                "No terminals in this chat yet.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(vertical = 48.dp).align(Alignment.CenterHorizontally),
            )
        } else {
            SelectionContainer(
                Modifier.fillMaxWidth().height(420.dp).clip(RoundedCornerShape(12.dp))
                    .background(shown?.let { rgba(it.background) } ?: MaterialTheme.colorScheme.surfaceContainer),
            ) {
                Text(
                    shown?.let(::paint) ?: AnnotatedString(""),
                    fontFamily = GeistMono,
                    fontSize = 12.sp,
                    lineHeight = 16.sp,
                    softWrap = false,
                    modifier = Modifier.verticalScroll(scroll).horizontalScroll(rememberScrollState()).padding(horizontal = 10.dp, vertical = 12.dp)
                        .testTag("terminal-output"),
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            "Output only. Type into this terminal on ${host ?: "its host"}.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun stateColor(state: RunState): Color = when (state) {
    RunState.RUNNING -> MaterialTheme.colorScheme.primary
    RunState.DONE -> successColor()
    RunState.FAILED -> MaterialTheme.colorScheme.error
}

private fun rgba(value: UInt): Color {
    val v = value.toLong()
    return Color(red = (v shr 24 and 0xFF).toInt(), green = (v shr 16 and 0xFF).toInt(), blue = (v shr 8 and 0xFF).toInt(), alpha = (v and 0xFF).toInt())
}

private fun paint(screen: AgentTerminalScreen): AnnotatedString = buildAnnotatedString {
    screen.lines.forEachIndexed { i, line ->
        if (i > 0) append('\n')
        for (span in line) {
            val style = SpanStyle(
                color = rgba(span.fg),
                background = span.bg?.let(::rgba) ?: Color.Unspecified,
                fontWeight = if (span.bold) FontWeight.SemiBold else null,
                fontStyle = if (span.italic) FontStyle.Italic else null,
                textDecoration = if (span.underline) TextDecoration.Underline else null,
            )
            pushStyle(style)
            append(span.text)
            pop()
        }
    }
}
