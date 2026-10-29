package sh.zeron.android.ui.session

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentedShapes
import sh.zeron.android.ui.successColor
import uniffi.zeron_core.AgentTerminalTab
import uniffi.zeron_core.ProjectActionItem
import uniffi.zeron_core.ProjectActions
import uniffi.zeron_core.RunState

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ProjectActionsSheet(extras: SessionExtras, onOpenTerminal: (terminalId: String?) -> Unit, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    var actions by remember { mutableStateOf<ProjectActions?>(null) }
    var loaded by remember { mutableStateOf(false) }
    val runs = remember { mutableStateMapOf<String, String>() }
    val tabs = remember { mutableStateMapOf<String, AgentTerminalTab>() }
    val starting = remember { mutableStateMapOf<String, Boolean>() }
    var following by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        try {
            actions = extras.handle.listProjectActions()
        } catch (e: Exception) {
            extras.model.toast("Couldn't load project actions. Try again.")
        }
        loaded = true
    }
    suspend fun refreshRuns() {
        val all = runCatching { extras.handle.listTerminals() }.getOrNull() ?: return
        val wanted = runs.values.toSet()
        val next = all.filter { it.id in wanted }.associateBy { it.id }
        if (next == tabs.toMap()) return
        tabs.clear()
        tabs.putAll(next)
    }
    LaunchedEffect(following) {
        if (!following) return@LaunchedEffect
        while (true) {
            delay(1000)
            refreshRuns()
            if (tabs.values.none { it.state == RunState.RUNNING }) break
        }
        following = false
    }
    fun run(item: ProjectActionItem) {
        starting[item.id] = true
        scope.launch {
            try {
                runs[item.id] = extras.handle.runProjectAction(item.id)
                refreshRuns()
                following = true
            } catch (e: Exception) {
                extras.model.toast("Couldn't run ${item.name}. Try again.")
            }
            starting.remove(item.id)
        }
    }

    val items = actions?.actions.orEmpty()
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp), contentPadding = PaddingValues(bottom = 32.dp)) {
            item("title") {
                Text("Project actions", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 8.dp, bottom = 12.dp))
            }
            if (items.isEmpty()) {
                item("empty") {
                    Text(
                        when {
                            !loaded -> "Loading…"
                            actions == null -> "This chat has no project."
                            else -> "This project has no actions yet. Add them on the Mac."
                        },
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 12.dp),
                    )
                }
            } else {
                itemsIndexed(items, key = { _, a -> a.id }) { i, item ->
                    val terminal = runs[item.id]
                    ActionRow(
                        item,
                        i,
                        items.size,
                        running = starting[item.id] == true || tabs[terminal]?.state == RunState.RUNNING,
                        tab = terminal?.let { tabs[it] },
                        onRun = { run(item) },
                        onOpen = terminal?.let { id -> { onOpenTerminal(id) } },
                    )
                }
            }
            val footer = actions?.issue ?: if (items.isEmpty()) null else "Each run opens a tab named after the action in Terminal."
            footer?.let {
                item("footer") {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 12.dp, end = 12.dp, top = 8.dp),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ActionRow(
    item: ProjectActionItem,
    index: Int,
    count: Int,
    running: Boolean,
    tab: AgentTerminalTab?,
    onRun: () -> Unit,
    onOpen: (() -> Unit)?,
) {
    SegmentedListItem(
        onClick = onOpen ?: {},
        enabled = onOpen != null,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor(), disabledContainerColor = cardColor()),
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).semantics { contentDescription = item.name },
        leadingContent = { ZIcon(actionIcon(item.icon), null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        supportingContent = {
            Text(item.command, fontFamily = GeistMono, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.onSurfaceVariant)
        },
        trailingContent = {
            when {
                running -> Row(verticalAlignment = Alignment.CenterVertically) {
                    LoadingIndicator(Modifier.size(24.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Running", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                }
                tab != null -> Text(
                    tab.status,
                    style = MaterialTheme.typography.labelLarge,
                    color = if (tab.state == RunState.DONE) successColor() else MaterialTheme.colorScheme.error,
                )
                else -> FilledTonalButton(
                    onClick = onRun,
                    contentPadding = PaddingValues(horizontal = 16.dp),
                    modifier = Modifier.heightIn(min = 36.dp).semantics { contentDescription = "Run ${item.name}" },
                ) { Text("Run") }
            }
        },
    ) {
        Text(item.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

private fun actionIcon(symbol: String): Int = when (symbol) {
    "checkmark.seal" -> ZIcons.Check
    "wand.and.stars" -> ZIcons.Magic
    "gearshape" -> ZIcons.Settings
    "hammer" -> ZIcons.Command
    "ladybug" -> ZIcons.Bot
    else -> ZIcons.Terminal
}
