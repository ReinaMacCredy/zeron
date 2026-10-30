package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
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
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.HostMeter
import uniffi.zeron_core.HostPage
import uniffi.zeron_core.HostRow
import uniffi.zeron_core.HostRowAction
import uniffi.zeron_core.HostRowKind
import uniffi.zeron_core.HostSection
import uniffi.zeron_core.HostTone

@Composable
fun HostPageScreen(model: AppModel, page: String, host: String?, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    val client by model.client.collectAsState()
    val workspace by model.workspace.collectAsState()
    val device = remember(client, workspace, host) {
        val hosts = client?.executionDevices().orEmpty()
        if (host != null) hosts.firstOrNull { it.name == host } else hosts.firstOrNull()
    }
    var pageId by remember(page) { mutableStateOf(page) }
    var shown by remember(page, device?.id) { mutableStateOf<HostPage?>(null) }
    var busy by remember { mutableStateOf(false) }
    var reload by remember { mutableIntStateOf(0) }
    var confirming by remember { mutableStateOf<HostRowAction?>(null) }
    var editing by remember { mutableStateOf<HostRow?>(null) }
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current

    fun error(detail: String) = HostPage(
        host ?: device?.name ?: "Mac",
        listOf(HostSection(null, null, listOf(HostRow("error", "Couldn't reach this Mac", detail, null, HostRowKind.Info, emptyList())))),
    )

    LaunchedEffect(client, device?.id, pageId, reload) {
        val c = client ?: return@LaunchedEffect
        val d = device ?: return@LaunchedEffect run { shown = error("No Mac in this workspace") }
        runCatching { c.hostPage(d.id, pageId) }
            .onSuccess { shown = it }
            .onFailure { if (shown == null) shown = error(it.message ?: "$it") }
    }

    fun act(id: String, value: String?) {
        val c = client ?: return
        val d = device ?: return
        if (busy) return
        busy = true
        scope.launch {
            runCatching { c.hostAction(d.id, pageId, id, value) }
                .onSuccess { result ->
                    result.pageId?.let { pageId = it }
                    if (result.page != null) shown = result.page else onBack()
                    result.openUrl?.let { runCatching { uri.openUri(it) } }
                }
                .onFailure {
                    model.toast(it.message ?: "$it")
                    reload++
                }
            busy = false
        }
    }

    fun confirm(action: HostRowAction) {
        if (action.confirm == null) act(action.id, null) else confirming = action
    }

    val current = shown
    SettingsPage(current?.title ?: device?.name ?: host ?: "Mac", onBack, busy = busy || current == null) {
        current?.sections?.forEach { section ->
            section.title?.let { settingsSection(it) }
            item {
                Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                    section.rows.forEachIndexed { i, row ->
                        HostRowItem(
                            row,
                            i,
                            section.rows.size,
                            onTap = {
                                when (val kind = row.kind) {
                                    is HostRowKind.Link -> onOpenRoute("page:${kind.page}@${device?.name ?: host ?: ""}")
                                    is HostRowKind.Check -> if (!kind.on) act(row.id, null)
                                    is HostRowKind.Action -> confirm(row.actions.firstOrNull { it.id == row.id } ?: HostRowAction(row.id, row.title, kind.destructive, null))
                                    is HostRowKind.Text -> editing = row
                                    is HostRowKind.Toggle -> act(row.id, if (kind.on) "false" else "true")
                                    else -> Unit
                                }
                            },
                            onToggle = { on -> act(row.id, if (on) "true" else "false") },
                            onAction = ::confirm,
                        )
                    }
                }
            }
            section.footer?.let { settingsFooter(it) }
        }
    }

    confirming?.let { action ->
        AlertDialog(
            onDismissRequest = { confirming = null },
            title = { Text(action.confirm ?: action.title) },
            confirmButton = {
                TextButton(onClick = {
                    confirming = null
                    act(action.id, null)
                }) { Text(action.title, color = if (action.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary) }
            },
            dismissButton = { TextButton(onClick = { confirming = null }) { Text("Cancel") } },
        )
    }
    editing?.let { row ->
        val kind = row.kind as HostRowKind.Text
        var text by remember(row.id) { mutableStateOf(row.value.orEmpty()) }
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text(row.title) },
            text = {
                OutlinedTextField(
                    text,
                    { text = it },
                    placeholder = kind.placeholder?.let { { Text(it) } },
                    singleLine = !kind.multiline,
                    minLines = if (kind.multiline) 4 else 1,
                    modifier = Modifier.fillMaxWidth(),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    editing = null
                    act(row.id, text)
                }) { Text("Save") }
            },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun HostRowItem(
    row: HostRow,
    index: Int,
    count: Int,
    onTap: () -> Unit,
    onToggle: (Boolean) -> Unit,
    onAction: (HostRowAction) -> Unit,
) {
    val kind = row.kind
    val extra = row.actions.filter { it.id != row.id }
    var menu by remember { mutableStateOf(false) }
    val accent = row.id.takeIf { it.startsWith("accent:#") }?.removePrefix("accent:#")?.toLongOrNull(16)?.let { Color(0xFF000000 or it) }
    Box {
        if (kind is HostRowKind.Meter) {
            SegmentedListItem(
                onClick = {},
                onLongClick = if (extra.isNotEmpty()) ({ menu = true }) else null,
                shapes = segmentedShapes(index, count),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            ) { MeterRow(row, kind.meter) }
        } else {
            val text = kind as? HostRowKind.Text
            val subtitle = if (text != null) row.value?.ifEmpty { null } ?: text.placeholder else row.subtitle
            SegmentedListItem(
                onClick = onTap,
                onLongClick = if (extra.isNotEmpty()) ({ menu = true }) else null,
                shapes = segmentedShapes(index, count),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                leadingContent = accent?.let { { Box(Modifier.size(22.dp).clip(CircleShape).background(it)) } },
                supportingContent = subtitle?.let { { Text(it, maxLines = if (text != null) 3 else Int.MAX_VALUE) } },
                trailingContent = {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (text == null) row.value?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        when (kind) {
                            is HostRowKind.Link, is HostRowKind.Text ->
                                ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline)
                            is HostRowKind.Check -> if (kind.on) ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                            is HostRowKind.Toggle -> Switch(kind.on, onToggle)
                            else -> Unit
                        }
                    }
                },
            ) {
                Text(
                    row.title,
                    color = when {
                        kind is HostRowKind.Action && kind.destructive -> MaterialTheme.colorScheme.error
                        kind is HostRowKind.Action -> MaterialTheme.colorScheme.primary
                        else -> MaterialTheme.colorScheme.onSurface
                    },
                )
            }
        }
        DropdownMenu(menu, { menu = false }) {
            extra.forEach { action ->
                DropdownMenuItem(
                    text = { Text(action.title, color = if (action.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface) },
                    onClick = {
                        menu = false
                        onAction(action)
                    },
                )
            }
        }
    }
}

@Composable
private fun MeterRow(row: HostRow, meter: HostMeter) {
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.padding(vertical = 4.dp)) {
        Row(verticalAlignment = Alignment.Bottom) {
            Text(row.title, style = MaterialTheme.typography.titleSmall)
            row.value?.let {
                Spacer(Modifier.width(6.dp))
                Text(it, style = MaterialTheme.typography.bodyMedium, color = secondary)
            }
            Spacer(Modifier.weight(1f))
            row.subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = secondary) }
        }
        Spacer(Modifier.height(8.dp))
        androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxWidth().heightIn(min = 12.dp), contentAlignment = Alignment.CenterStart) {
            val track = MaterialTheme.colorScheme.outline.copy(alpha = 0.25f)
            Box(Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(track))
            Box(Modifier.width(maxWidth * meter.fill.coerceIn(0f, 1f)).height(6.dp).clip(RoundedCornerShape(3.dp)).background(toneColor(meter.tone)))
            meter.marker?.let { m ->
                Box(Modifier.offset(x = maxWidth * m.coerceIn(0f, 1f) - 1.dp).width(2.dp).height(12.dp).background(toneColor(meter.markerTone)))
            }
        }
        if (meter.footLeft != null || meter.footRight != null) {
            Spacer(Modifier.height(6.dp))
            Row {
                meter.footLeft?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = if (meter.footLeftAlert) MaterialTheme.colorScheme.error else secondary) }
                Spacer(Modifier.weight(1f))
                meter.footRight?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = if (meter.footRightAlert) MaterialTheme.colorScheme.error else secondary) }
            }
        }
    }
}

@Composable
private fun toneColor(tone: HostTone): Color = when (tone) {
    HostTone.NORMAL -> MaterialTheme.colorScheme.primary
    HostTone.WARN -> if (LocalDarkTheme.current) Color(0xFFFACC15) else Color(0xFFA16207)
    HostTone.CRITICAL -> MaterialTheme.colorScheme.error
}
