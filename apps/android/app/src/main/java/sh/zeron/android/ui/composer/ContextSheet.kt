package sh.zeron.android.ui.composer

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import sh.zeron.android.ui.chipContainer
import sh.zeron.android.ui.warningTint
import uniffi.zeron_core.ContextCard
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.StatsRow

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ContextSheet(
    handle: SessionHandle,
    card: (ULong?) -> ContextCard?,
    onToggleLong: () -> Unit,
    onPick: (UInt) -> Unit,
    onDismiss: () -> Unit,
) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1_000)
            now = System.currentTimeMillis()
        }
    }
    val stats = handle.statsView(now)
    val compact = handle.compactSection(now)
    val context = card(handle.composer().contextUsage?.tokens)
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp).testTag("context-sheet")) {
            Text("Context and stats", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 8.dp, bottom = 8.dp))
            context?.let { c ->
                Section("Context", c.warning, footerWarn = true) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text("Long context", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                        Switch(c.longContext, { onToggleLong() }, modifier = Modifier.testTag("context-long"))
                    }
                    val stops = listOf(c.shortLabel) + c.stops
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).testTag("context-track")) {
                        stops.forEachIndexed { i, label ->
                            SegmentedButton(
                                selected = i == c.position.toInt(),
                                onClick = { onPick(i.toUInt()) },
                                shape = SegmentedButtonDefaults.itemShape(i, stops.size),
                                icon = {},
                            ) { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        }
                    }
                    Text(
                        "${c.windowLabel} · ${c.thresholdLabel}",
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (c.longContext) MaterialTheme.colorScheme.onSurfaceVariant else warningTint(),
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
                    )
                }
            }
            compact?.let { Section(it.heading, it.note) { it.rows.forEach { r -> Stat(r) } } }
            stats?.cost?.let { cost ->
                Section("Session cost", stats.costNote) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp).testTag("stats-cost"), verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            cost,
                            style = if (stats.priced) MaterialTheme.typography.titleLarge else MaterialTheme.typography.bodyLarge,
                            color = if (stats.priced) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f),
                        )
                        Text(stats.tokens, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    stats.split.forEach { Stat(it) }
                }
            }
            stats?.let { s -> Section("Session statistics", null) { s.rows.forEach { Stat(it) } } }
        }
    }
}

@Composable
private fun Section(title: String, footer: String?, footerWarn: Boolean = false, content: @Composable () -> Unit) {
    Spacer(Modifier.height(12.dp))
    Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 16.dp, bottom = 6.dp))
    Surface(shape = MaterialTheme.shapes.large, color = chipContainer()) {
        Column(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(0.dp)) { content() }
    }
    footer?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = if (footerWarn) warningTint() else MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 6.dp))
    }
}

@Composable
private fun Stat(row: StatsRow) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(row.label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Text(row.value, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
