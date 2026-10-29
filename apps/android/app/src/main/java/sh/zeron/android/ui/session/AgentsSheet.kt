package sh.zeron.android.ui.session

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.ShellRow

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun AgentsSheet(
    model: AppModel,
    chatId: String,
    extras: SessionExtras,
    onOpenChat: (String) -> Unit,
    onOpenSubagent: (docId: String, title: String, running: Boolean) -> Unit,
    onOpenTerminal: (terminalId: String?) -> Unit,
    onOpenFindings: (taskId: String?) -> Unit,
    onDismiss: () -> Unit,
) {
    val handle = extras.handle
    val roster = rememberRoster(model, chatId, handle, ticking = true)
    val r = roster.value
    val sheet = rememberModalBottomSheetState()
    val scope = rememberCoroutineScope()
    var shell by remember { mutableStateOf<ShellRow?>(null) }
    fun dismissThen(run: () -> Unit) {
        scope.launch { sheet.hide() }.invokeOnCompletion {
            onDismiss()
            run()
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 32.dp), modifier = Modifier.fillMaxWidth()) {
            item("title") {
                Column(Modifier.padding(start = 8.dp, bottom = 4.dp)) {
                    Text(r?.title ?: "Agents", style = MaterialTheme.typography.headlineSmallEmphasized)
                    r?.summary?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
            if (r == null || (r.agents.isEmpty() && r.shells.isEmpty())) {
                item("empty") {
                    Text(
                        "Nothing is running in the background.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 8.dp, top = 16.dp),
                    )
                }
                return@LazyColumn
            }
            if (r.agents.isNotEmpty()) {
                item("h-agents") { SheetHeader(r.agentsSection) }
                itemsIndexed(r.agents, key = { _, a -> "a/${a.taskId}" }) { i, run ->
                    SegmentedListItem(
                        onClick = { openRun(model, chatId, run, onOpenChat, onOpenSubagent, onOpenFindings) { go -> if (run.review != null) go() else dismissThen(go) } },
                        shapes = segmentedShapes(i, r.agents.size),
                        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                        leadingContent = { RunGlyph(run.state, Modifier.padding(start = 16.dp * run.depth.toInt())) },
                        supportingContent = { Text(listOfNotNull(run.line, run.review).joinToString("\n")) },
                        trailingContent = when {
                            run.canStop -> ({ StopButton("Stop ${run.title}") { stopRun(model, handle, scope, run.taskId, run.delegated, roster::reload) } })
                            run.delegated || run.review != null -> ({ ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) })
                            else -> null
                        },
                        modifier = Modifier.animateItem().padding(bottom = ListItemDefaults.SegmentedGap),
                    ) { Text(run.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
            }
            if (r.shells.isNotEmpty()) {
                item("h-shells") { SheetHeader(r.shellsSection) }
                itemsIndexed(r.shells, key = { _, s -> "s/${s.taskId}" }) { i, s ->
                    SegmentedListItem(
                        onClick = { shell = s },
                        shapes = segmentedShapes(i, r.shells.size),
                        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                        leadingContent = { ZIcon(ZIcons.Terminal, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
                        supportingContent = { Text(s.line) },
                        trailingContent = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) },
                        modifier = Modifier.animateItem().padding(bottom = ListItemDefaults.SegmentedGap),
                    ) { Text(s.label, style = MaterialTheme.typography.titleMedium, fontFamily = GeistMono, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
            }
        }
    }
    shell?.let { s ->
        ShellOutputSheet(handle, s, onStop = { stopRun(model, handle, scope, s.taskId, false, roster::reload) }) { shell = null }
    }
}

@Composable
private fun SheetHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmallEmphasized,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
    )
}
