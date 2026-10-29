package sh.zeron.android.ui.session

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
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
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.git.DiffDelta
import sh.zeron.android.ui.git.color
import sh.zeron.android.ui.git.rememberDiffPalette
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.ChatChangesView
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.InfoSource
import uniffi.zeron_core.UsageLevel
import uniffi.zeron_core.UsageMeter

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionInfoSheet(
    extras: SessionExtras,
    sources: List<InfoSource>,
    onOpenChanges: (title: String) -> Unit,
    onOpenImage: (reference: String) -> Unit,
    onDismiss: () -> Unit,
) {
    var usage by remember { mutableStateOf<UsageMeter?>(null) }
    var usageLoaded by remember { mutableStateOf(false) }
    var changes by remember { mutableStateOf<ChatChangesView?>(null) }
    LaunchedEffect(Unit) {
        usage = runCatching { extras.core.chatUsage(extras.chatId) }.getOrNull()
        usageLoaded = true
    }
    LaunchedEffect(Unit) {
        if (extras.handle.composer().review != null) changes = runCatching { extras.handle.chatChanges() }.getOrNull()
    }
    ModalBottomSheet(onDismissRequest = onDismiss, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp), contentPadding = PaddingValues(bottom = 32.dp)) {
            item("title") {
                Text("Session info", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 8.dp, bottom = 12.dp))
            }
            changes?.let { c ->
                item("changes") {
                    InfoRow(ZIcons.Branch, c.title, 0, 1, onClick = { onOpenChanges(c.title) }) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("${c.files} ", fontFamily = GeistMono, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            DiffDelta(c.additions, c.deletions)
                        }
                    }
                }
            }
            if (sources.isNotEmpty()) {
                header("Sources")
                itemsIndexed(sources, key = { i, s -> "source/$i/${s.reference}" }) { i, s ->
                    InfoRow(ZIcons.Text, s.name, i, sources.size, onClick = { onOpenImage(s.reference) })
                }
            }
            header("Usage")
            item("usage") {
                val meter = usage
                InfoRow(
                    ZIcons.Context,
                    meter?.label ?: if (usageLoaded) "Usage unavailable" else "Loading usage…",
                    0,
                    1,
                    secondary = meter == null,
                ) { meter?.let { UsageBar(it) } }
            }
        }
    }
}

private fun LazyListScope.header(title: String) {
    item("h-$title") {
        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun InfoRow(
    icon: Int,
    text: String,
    index: Int,
    count: Int,
    secondary: Boolean = false,
    onClick: (() -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
) {
    SegmentedListItem(
        onClick = onClick ?: {},
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap),
        leadingContent = { ZIcon(icon, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        trailingContent = trailing,
    ) {
        Text(
            text,
            style = MaterialTheme.typography.bodyLarge,
            color = if (secondary) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            maxLines = 1,
            overflow = TextOverflow.MiddleEllipsis,
        )
    }
}

@Composable
private fun UsageBar(usage: UsageMeter) {
    val palette = rememberDiffPalette()
    val tone = when (usage.level) {
        UsageLevel.NORMAL -> MaterialTheme.colorScheme.primary
        UsageLevel.WARN -> palette.color(ColorRole.WARNING)
        UsageLevel.CRITICAL -> palette.color(ColorRole.DANGER)
    }
    LinearProgressIndicator(
        progress = { usage.fraction.coerceIn(0f, 1f) },
        modifier = Modifier.width(96.dp).height(4.dp).clip(RoundedCornerShape(2.dp)),
        color = tone,
        trackColor = MaterialTheme.colorScheme.outlineVariant,
        strokeCap = StrokeCap.Round,
        gapSize = 0.dp,
        drawStopIndicator = {},
    )
}
