package sh.zeron.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.TonalCircleButton
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.chipContainer
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.ConfigFilter
import uniffi.zeron_core.ConfigRowKind
import uniffi.zeron_core.ConfigRowView
import uniffi.zeron_core.ConfigTab

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ClaudeConfigSheet(
    model: AppModel,
    chatId: String,
    extras: SessionExtras,
    tab: ConfigTab = ConfigTab.CONFIG,
    query: String = "",
    onOpenUsage: () -> Unit = {},
    onDismiss: () -> Unit,
) {
    if (!extras.runsClaudeCode) {
        LaunchedEffect(Unit) { onDismiss() }
        return
    }
    val panel = remember(chatId) { extras.core.claudeConfigPanel(chatId) }
    val state = remember(panel) { ConfigSheetState(tab, query) }
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    var finished by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf<ConfigRowView?>(null) }

    fun finish() {
        if (finished) return
        finished = true
        extras.launch("Couldn't add the settings summary. Try again.") {
            try {
                panel.closePanel()
            } catch (e: Exception) {
                model.toast("Couldn't add the settings summary: ${SessionExtras.failure(e, e.message ?: e.toString())}")
            }
        }
    }

    fun dismiss(then: () -> Unit = {}) {
        scope.launch {
            sheet.hide()
            finish()
            onDismiss()
            then()
        }
    }

    fun requestStats() {
        if (state.statsRequested) return
        state.statsRequested = true
        scope.launch {
            panel.loadStats()
            state.rev++
        }
    }

    fun change(row: ConfigRowView, value: String) {
        haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
        extras.launch("Couldn't change the setting. Try again.") {
            val pending = scope.launch {
                delay(30)
                state.rev++
            }
            val chip = panel.change(row.id, value)
            pending.cancel()
            if (chip != null) {
                extras.applyChip(chip)
                panel.refreshChips()
            }
            state.rev++
        }
    }

    fun activate(row: ConfigRowView) {
        state.selected = row.id
        state.rev++
        if (!row.writable) return
        when (row.kind) {
            ConfigRowKind.TOGGLE, ConfigRowKind.CYCLE -> row.nextValue?.let { change(row, it) }
            ConfigRowKind.OPTIONS -> state.expanded = if (state.expanded == row.id) null else row.id
            ConfigRowKind.TEXT -> editing = row
        }
    }

    LaunchedEffect(panel) {
        panel.load()
        state.rev++
        if (state.tab == ConfigTab.STATS) requestStats()
    }

    ModalBottomSheet(
        onDismissRequest = {
            finish()
            onDismiss()
        },
        sheetState = sheet,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        val tabs = remember(panel) { panel.tabs() }
        val list = remember(state.rev, state.filter, state.query) { panel.list(state.filter, state.query) }
        Column(Modifier.fillMaxHeight(0.92f).semantics { contentDescription = "Claude Code settings" }) {
            Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Claude Code settings", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.weight(1f))
                TonalCircleButton(ZIcons.Close, "Close", onClick = { dismiss() }, size = 40.dp, container = MaterialTheme.colorScheme.surfaceContainerHighest)
            }
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 14.dp)) {
                tabs.forEachIndexed { i, info ->
                    SegmentedButton(
                        selected = state.tab == info.tab,
                        onClick = {
                            state.tab = info.tab
                            state.expanded = null
                            if (info.tab == ConfigTab.STATS) requestStats()
                        },
                        shape = SegmentedButtonDefaults.itemShape(i, tabs.size),
                        icon = {},
                        label = { Text(info.label, maxLines = 1) },
                    )
                }
            }
            if (state.tab == ConfigTab.CONFIG) {
                OutlinedTextField(
                    state.query,
                    {
                        state.query = it
                        state.expanded = null
                    },
                    placeholder = { Text("Search settings…") },
                    leadingIcon = { ZIcon(ZIcons.Search, null, Modifier.size(20.dp)) },
                    singleLine = true,
                    shape = CircleShape,
                    colors = OutlinedTextFieldDefaults.colors(
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                        unfocusedBorderColor = Color.Transparent,
                    ),
                    modifier = Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 10.dp),
                )
                Row(
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    for (chip in remember(panel) { panel.filters() }) {
                        FilterCapsule("${chip.label} ${chip.count}", chip.filter == state.filter) {
                            state.filter = chip.filter
                            state.expanded = null
                        }
                    }
                    Spacer(Modifier.width(4.dp))
                    Text(list.shown, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
                }
            } else {
                Spacer(Modifier.size(12.dp))
            }
            LazyColumn(
                contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 28.dp),
                modifier = Modifier.weight(1f),
            ) {
                state.rev
                when (state.tab) {
                    ConfigTab.CONFIG -> configItems(list.message, list.rows, list.footer, state, onActivate = ::activate, onPick = { row, option ->
                        state.expanded = null
                        change(row, option)
                    })
                    ConfigTab.STATUS -> statusItems(panel)
                    ConfigTab.USAGE -> usageItems(panel) { dismiss(onOpenUsage) }
                    ConfigTab.STATS -> statsItems(panel)
                }
            }
        }
    }

    editing?.let { row ->
        var text by remember(row.id) { mutableStateOf(row.value) }
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text(row.label) },
            text = {
                Column {
                    Text(row.key, style = MaterialTheme.typography.bodySmall.copy(fontFamily = GeistMono), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.size(12.dp))
                    OutlinedTextField(text, { text = it }, placeholder = { Text("e.g. Vietnamese") }, singleLine = true)
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    editing = null
                    change(row, text.trim())
                }) { Text("Save") }
            },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
    }
}

internal class ConfigSheetState(tab: ConfigTab, query: String) {
    var tab by mutableStateOf(tab)
    var filter by mutableStateOf(ConfigFilter.ALL)
    var query by mutableStateOf(query)
    var expanded by mutableStateOf<String?>(null)
    var selected by mutableStateOf<String?>(null)
    var statsRequested = false
    var rev by mutableIntStateOf(0)
}

@Composable
private fun FilterCapsule(text: String, on: Boolean, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = if (on) scheme.primary else scheme.onSurfaceVariant,
        maxLines = 1,
        modifier = Modifier
            .clip(CircleShape)
            .background(if (on) scheme.primaryContainer.copy(alpha = 0.6f) else chipContainer())
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .semantics { contentDescription = "$text${if (on) ", selected" else ""}" },
    )
}

private sealed interface ConfigLine {
    data class Setting(val row: ConfigRowView) : ConfigLine
    data class Option(val row: ConfigRowView, val option: String) : ConfigLine
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
private fun LazyListScope.configItems(
    message: String?,
    rows: List<ConfigRowView>,
    footer: String,
    state: ConfigSheetState,
    onActivate: (ConfigRowView) -> Unit,
    onPick: (ConfigRowView, String) -> Unit,
) {
    if (message != null) {
        item("message") { ConfigMessage(message) }
    } else {
        val lines = rows.flatMap { row ->
            if (state.expanded == row.id) listOf(ConfigLine.Setting(row)) + row.options.map { ConfigLine.Option(row, it) } else listOf(ConfigLine.Setting(row))
        }
        itemsIndexed(lines, key = { _, l -> if (l is ConfigLine.Option) "${l.row.id}=${l.option}" else (l as ConfigLine.Setting).row.id }) { i, line ->
            when (line) {
                is ConfigLine.Setting -> SettingRow(line.row, i, lines.size, state, onActivate)
                is ConfigLine.Option -> OptionRow(line.row, line.option, i, lines.size) { onPick(line.row, line.option) }
            }
        }
    }
    if (footer.isNotEmpty()) item("footer") { Footnote(footer) }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SettingRow(row: ConfigRowView, index: Int, count: Int, state: ConfigSheetState, onActivate: (ConfigRowView) -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val selected = state.selected == row.id
    SegmentedListItem(
        onClick = { onActivate(row) },
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = if (selected) scheme.primaryContainer.copy(alpha = 0.45f) else cardColor()),
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).semantics { contentDescription = "claude-config-row-${row.id}" },
        supportingContent = {
            Column {
                Text(highlighted(row.key, state.query), style = MaterialTheme.typography.bodySmall.copy(fontFamily = GeistMono, fontSize = 11.5.sp), color = scheme.outline)
                val note = row.note
                if (note != null && (selected || row.notePinned)) {
                    Text(note, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, modifier = Modifier.padding(top = 2.dp))
                }
            }
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (row.changed) Box(Modifier.size(6.dp).clip(CircleShape).background(scheme.primary))
                if (row.kind == ConfigRowKind.TOGGLE) {
                    Switch(checked = row.value == "true", onCheckedChange = { onActivate(row) }, enabled = row.writable)
                } else {
                    Text(
                        highlighted(row.value.ifEmpty { "default" }, state.query),
                        style = MaterialTheme.typography.bodyMedium.copy(fontFamily = GeistMono, fontSize = 13.sp),
                        color = if (row.value.isEmpty() || !row.writable) scheme.outline else scheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.widthIn(max = 150.dp),
                    )
                    if (row.kind == ConfigRowKind.OPTIONS && row.writable) {
                        ZIcon(if (state.expanded == row.id) ZIcons.ChevronUp else ZIcons.ChevronDown, null, Modifier.size(14.dp), tint = scheme.outline)
                    }
                }
                row.tag?.let { ConfigTag(it, row.chip) }
            }
        },
    ) {
        Text(
            highlighted(row.label, state.query),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Medium,
            color = if (row.writable) scheme.onSurface else scheme.outline,
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun OptionRow(row: ConfigRowView, option: String, index: Int, count: Int, onClick: () -> Unit) {
    SegmentedListItem(
        onClick = onClick,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).semantics { contentDescription = "claude-config-option-$option" },
        trailingContent = if (option == row.value) {
            { ZIcon(ZIcons.Check, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary) }
        } else {
            null
        },
    ) {
        Text(option, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = GeistMono, fontSize = 13.5.sp), modifier = Modifier.padding(start = 16.dp))
    }
}

@Composable
internal fun ConfigTag(text: String, chip: Boolean) {
    val scheme = MaterialTheme.colorScheme
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = if (chip) scheme.primary else scheme.onSurfaceVariant,
        maxLines = 1,
        modifier = Modifier
            .clip(RoundedCornerShape(5.dp))
            .background(if (chip) scheme.primaryContainer.copy(alpha = 0.6f) else chipContainer())
            .padding(horizontal = 6.dp, vertical = 2.dp),
    )
}

@Composable
internal fun ConfigMessage(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.outline,
        textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 32.dp).semantics { contentDescription = "claude-config-message" },
    )
}

@Composable
internal fun Footnote(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp),
    )
}

private fun highlighted(text: String, query: String): AnnotatedString {
    val q = query.trim()
    val at = if (q.isEmpty()) -1 else text.indexOf(q, ignoreCase = true)
    if (at < 0) return AnnotatedString(text)
    return buildAnnotatedString {
        append(text)
        addStyle(SpanStyle(background = ConfigWarning.copy(alpha = 0.3f)), at, at + q.length)
    }
}

internal val ConfigWarning = Color(0xFFD97706)
