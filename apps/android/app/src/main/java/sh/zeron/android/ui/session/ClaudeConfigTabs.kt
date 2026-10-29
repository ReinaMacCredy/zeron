package sh.zeron.android.ui.session

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.design.GeistMono
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.chipContainer
import uniffi.zeron_core.ClaudeConfigPanel
import uniffi.zeron_core.ConfigStatusRow
import uniffi.zeron_core.StatTile
import uniffi.zeron_core.UsageMeterView

private val GroupShape = RoundedCornerShape(20.dp)

@Composable
private fun Group(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Surface(shape = GroupShape, color = cardColor(), modifier = modifier.fillMaxWidth().padding(bottom = 2.dp)) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) { content() }
    }
}

internal fun LazyListScope.statusItems(panel: ClaudeConfigPanel) {
    val status = panel.status()
    itemsIndexed(status.rows, key = { _, r -> r.key }) { _, row -> StatusRow(row) }
    item("footnote") { Footnote(status.footnote) }
}

@Composable
private fun StatusRow(row: ConfigStatusRow) {
    Group(Modifier.semantics { contentDescription = "claude-config-status-${row.key}" }) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(row.key, style = MaterialTheme.typography.bodyMedium, fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(row.value, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = GeistMono, fontSize = 13.sp), color = MaterialTheme.colorScheme.onSurface)
            }
            if (row.chip) ConfigTag("= chip", chip = true)
        }
    }
}

internal fun LazyListScope.usageItems(panel: ClaudeConfigPanel, onOpenUsage: () -> Unit) {
    val report = panel.usage()
    report.message?.let { message ->
        item("message") { ConfigMessage(message) }
        return
    }
    itemsIndexed(report.meters, key = { _, m -> "meter-${m.label}" }) { _, meter -> Meter(meter) }
    itemsIndexed(report.sections, key = { i, _ -> "section-$i" }) { _, section -> TextGroup(section.title, section.lines) }
    report.note?.let { note -> item("note") { TextGroup(note, emptyList()) } }
    item("open-usage") {
        Group {
            OutlinedButton(onClick = onOpenUsage, modifier = Modifier.semantics { contentDescription = "claude-config-open-usage" }) { Text("Open Usage page") }
        }
    }
}

@Composable
private fun Meter(meter: UsageMeterView) {
    val scheme = MaterialTheme.colorScheme
    val tone = if (meter.warning) ConfigWarning else scheme.primary
    Group(Modifier.semantics { contentDescription = "claude-config-meter-${meter.label}" }) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(meter.label, style = MaterialTheme.typography.bodyLarge, fontSize = 14.sp, modifier = Modifier.weight(1f))
            Text("${meter.percent}%", style = MaterialTheme.typography.bodyLarge, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = if (meter.warning) ConfigWarning else scheme.onSurfaceVariant)
        }
        Spacer(Modifier.height(6.dp))
        Box(Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)).background(chipContainer())) {
            Box(Modifier.fillMaxWidth(meter.percent.coerceAtMost(100u).toFloat().div(100f).coerceAtLeast(0.02f)).fillMaxHeight().clip(RoundedCornerShape(3.dp)).background(tone))
        }
        meter.resets?.let {
            Spacer(Modifier.height(6.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = scheme.outline)
        }
    }
}

@Composable
private fun TextGroup(title: String, lines: List<String>) {
    Group {
        Text(title, style = MaterialTheme.typography.bodyLarge, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        if (lines.isNotEmpty()) {
            Text(lines.joinToString("\n"), style = MaterialTheme.typography.bodySmall, fontSize = 12.5.sp, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 2.dp))
        }
    }
}

internal fun LazyListScope.statsItems(panel: ClaudeConfigPanel) {
    val stats = panel.stats()
    val message = stats.message
    if (message != null) {
        item("message") { ConfigMessage(message) }
    } else {
        item("heat") { Heat(stats.caption, stats.heat) }
        item("tiles") { Tiles(stats.tiles) }
    }
    item("footnote") { Footnote(stats.footnote) }
}

private const val CELL = 7
private const val GAP = 2

@Composable
private fun Heat(caption: String, heat: ByteArray) {
    val scheme = MaterialTheme.colorScheme
    val empty = chipContainer()
    val accent = scheme.primary
    val weeks = (heat.size + 6) / 7
    Group(Modifier.semantics { contentDescription = "claude-config-heat" }) {
        Text(caption, style = MaterialTheme.typography.bodySmall, color = scheme.outline)
        Spacer(Modifier.height(8.dp))
        Box(Modifier.horizontalScroll(rememberScrollState())) {
            Canvas(Modifier.size(((CELL + GAP) * weeks - GAP).dp, (CELL * 7 + GAP * 6).dp)) {
                val cell = CELL.dp.toPx()
                val step = (CELL + GAP).dp.toPx()
                val radius = CornerRadius(1.5.dp.toPx())
                heat.forEachIndexed { i, level ->
                    val l = (level.toInt() and 0xFF).coerceAtMost(4)
                    drawRoundRect(
                        color = if (l == 0) empty else accent.copy(alpha = floatArrayOf(0f, 0.25f, 0.45f, 0.7f, 1f)[l]),
                        topLeft = Offset((i / 7) * step, (i % 7) * step),
                        size = Size(cell, cell),
                        cornerRadius = radius,
                    )
                }
            }
        }
    }
}

@Composable
private fun Tiles(tiles: List<StatTile>) {
    val border = chipContainer()
    Group(Modifier.semantics { contentDescription = "claude-config-tiles" }) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (pair in tiles.chunked(2)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (tile in pair) {
                        Column(Modifier.weight(1f).border(1.dp, border, RoundedCornerShape(10.dp)).padding(horizontal = 10.dp, vertical = 8.dp)) {
                            Text(tile.value, style = MaterialTheme.typography.bodyLarge, fontSize = 15.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(tile.label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline, maxLines = 1)
                        }
                    }
                    if (pair.size == 1) Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}
