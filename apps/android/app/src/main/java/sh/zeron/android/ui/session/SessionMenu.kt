package sh.zeron.android.ui.session

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.BranchOffer
import uniffi.zeron_core.HandoffTarget
import uniffi.zeron_core.LimitPill
import uniffi.zeron_core.MissionStrip

class SessionMenuItem(
    val label: String,
    @androidx.annotation.DrawableRes val icon: Int,
    val subtitle: String? = null,
    val enabled: Boolean = true,
    val destructive: Boolean = false,
    val checked: Boolean? = null,
    val onClick: () -> Unit,
)

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionMenu(expanded: Boolean, onDismiss: () -> Unit, groups: List<List<SessionMenuItem>>) {
    val shown = groups.filter { it.isNotEmpty() }
    DropdownMenuPopup(expanded = expanded, onDismissRequest = onDismiss) {
        shown.forEachIndexed { g, items ->
            if (g > 0) Spacer(Modifier.size(MenuDefaults.GroupSpacing))
            DropdownMenuGroup(shapes = MenuDefaults.groupShape(g, shown.size)) {
                items.forEachIndexed { i, a ->
                    val tint = when {
                        !a.enabled -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                        a.destructive -> MaterialTheme.colorScheme.error
                        else -> MaterialTheme.colorScheme.onSurface
                    }
                    DropdownMenuItem(
                        onClick = {
                            onDismiss()
                            a.onClick()
                        },
                        enabled = a.enabled,
                        text = {
                            Column {
                                Text(a.label, color = tint)
                                a.subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2) }
                            }
                        },
                        shape = MenuDefaults.itemShape(i, items.size).shape,
                        leadingIcon = { ZIcon(a.icon, null, Modifier.size(20.dp), tint = tint) },
                        trailingIcon = if (a.checked == true) {
                            { ZIcon(ZIcons.Check, "On", Modifier.size(20.dp), tint = tint) }
                        } else {
                            null
                        },
                    )
                }
            }
        }
    }
}

@Composable
fun BranchDialog(offer: BranchOffer, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { ZIcon(if (offer.rewind) ZIcons.Restart else ZIcons.Fork, null) },
        title = { Text(offer.title) },
        text = { Text("${offer.context}\n\n${offer.detail}") },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                onConfirm()
            }) { Text(offer.option, color = if (offer.rewind) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
fun StopComputerDialog(host: String, onStop: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Stop computer use?") },
        text = { Text("Zeron stops using apps on $host for this chat now. Other chats keep their access.") },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                onStop()
            }) { Text("Stop", color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
fun CompactOfferDialog(title: String, body: String, onCompact: () -> Unit, onKeep: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { Text(body) },
        confirmButton = {
            TextButton(onClick = {
                onDismiss()
                onCompact()
            }) { Text("Compact") }
        },
        dismissButton = {
            TextButton(onClick = {
                onDismiss()
                onKeep()
            }) { Text("Keep full history") }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LimitActionsSheet(limit: LimitPill, onSwitch: () -> Unit, onToggleResume: () -> Unit, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Text(limit.text, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
        limit.switchTo?.let { model -> SheetRow("Switch to $model", ZIcons.Refresh) { onDismiss(); onSwitch() } }
        limit.action?.let { action -> SheetRow(action, ZIcons.Restart) { onDismiss(); onToggleResume() } }
        SheetRow("Close", ZIcons.Close, onClick = onDismiss)
        Spacer(Modifier.size(24.dp))
    }
}

@Composable
private fun SheetRow(label: String, @androidx.annotation.DrawableRes icon: Int, subtitle: String? = null, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(label) },
        supportingContent = subtitle?.let { { Text(it) } },
        leadingContent = { ZIcon(icon, null, Modifier.size(22.dp)) },
        colors = ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun HandoffSheet(extras: SessionExtras, onOpenChat: (String) -> Unit, onDismiss: () -> Unit) {
    val labels = remember { uniffi.zeron_core.handoffLabels() }
    val targets by produceState<List<HandoffTarget>?>(null) {
        value = runCatching { extras.core.handoffTargets(extras.hostDevice) }.getOrDefault(emptyList())
    }
    val row = extras.model.row(extras.chatId)
    val current = extras.core.sessionConfig(extras.chatId)?.harness ?: row?.harness
    val projects = if (row != null && row.project == null) extras.model.workspace.value?.projects.orEmpty().filter { it.deviceId == row.deviceId } else emptyList()
    fun title(t: HandoffTarget) = "${t.name} · ${t.modelLabel}"
    fun fresh(harness: String?, model: String?, spaceId: String?) {
        onDismiss()
        extras.launch("") {
            try {
                onOpenChat(extras.core.continueInNewThread(extras.chatId, harness, model, spaceId))
            } catch (e: Exception) {
                extras.model.toast(uniffi.zeron_core.handoffFailedMessage(e.message ?: e.toString()))
            }
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Text(labels.title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
        val list = targets
        if (list == null) {
            Row(Modifier.fillMaxWidth().padding(24.dp), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.Center) { LoadingIndicator(Modifier.size(32.dp)) }
            return@ModalBottomSheet
        }
        LazyColumn(Modifier.fillMaxWidth()) {
            val inPlace = list.filter { it.harness != current }
            if (inPlace.isNotEmpty()) {
                item { SectionLabel(labels.thisThread) }
                items(inPlace) { t ->
                    SheetRow(title(t), ZIcons.Bot) {
                        onDismiss()
                        extras.confirmSwitch(
                            extras.core.providerSwitchPrompt(extras.chatId, row?.harnessLabel ?: row?.harness ?: "", t.name, row?.modelLabel ?: row?.model ?: "", t.modelLabel),
                        ) { it.copy(harness = t.harness, model = t.model, reasoning = null, modelOptions = emptyMap(), profileId = null) }
                    }
                }
            }
            if (list.isNotEmpty()) {
                item { SectionLabel(labels.newThread) }
                items(list) { t -> SheetRow(title(t), ZIcons.NewSession) { fresh(t.harness, t.model, null) } }
            }
            if (projects.isNotEmpty()) {
                item { SectionLabel(labels.project) }
                items(projects) { p -> SheetRow(p.name, ZIcons.Folder) { fresh(null, null, p.id) } }
            }
            if (list.isEmpty() && projects.isEmpty()) item { SectionLabel("No other agents on this host") }
            item { Spacer(Modifier.size(24.dp)) }
        }
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(text, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 12.dp, bottom = 4.dp))
}

@Composable
fun MissionStripPill(strip: MissionStrip, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.92f),
        border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = modifier.semantics { contentDescription = "Mission: ${strip.title}, ${strip.done} of ${strip.total}" },
    ) {
        Row(Modifier.padding(horizontal = 12.dp, vertical = 7.dp), verticalAlignment = Alignment.CenterVertically) {
            ZIcon(ZIcons.TabTasks, null, Modifier.size(14.dp), tint = MaterialTheme.colorScheme.onSurface)
            Spacer(Modifier.size(6.dp))
            Text(
                "${strip.role} · ${strip.title} · ${strip.done}/${strip.total}",
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
