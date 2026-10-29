package sh.zeron.android.ui.review

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.git.DiffSheet
import sh.zeron.android.ui.git.FindingAction
import sh.zeron.android.ui.git.color
import sh.zeron.android.ui.git.rememberDiffPalette
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.ReviewFindingRow
import uniffi.zeron_core.ReviewFindingState
import uniffi.zeron_core.ReviewSeverity

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReviewFindingsSheet(extras: SessionExtras, taskId: String?, onDismiss: () -> Unit) {
    val handle = extras.handle
    var local by remember { mutableIntStateOf(0) }
    val revision = rememberReviewRevision(extras)
    var latest by remember { mutableStateOf(handle.reviewFindings(taskId)) }
    LaunchedEffect(revision, local) { handle.reviewFindings(taskId)?.let { latest = it } }
    var diff by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) { if (latest == null) onDismiss() }
    val findings = latest ?: return
    fun dismissFinding(id: String) {
        handle.dismissReviewFinding(id)
        local++
    }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        LazyColumn(Modifier.fillMaxWidth().testTag("review-findings")) {
            item { SheetTitle(findings.title) }
            val header = listOfNotNull(findings.summary, findings.note, findings.retryError).joinToString("\n")
            if (header.isNotEmpty()) {
                item {
                    Text(
                        header,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 20.dp).padding(bottom = 12.dp),
                    )
                }
            }
            itemsIndexed(findings.rows, key = { _, r -> r.id }) { i, row ->
                FindingRow(row, i, findings.rows.size, onOpen = { diff = row.id }, onDismiss = { dismissFinding(row.id) })
            }
            item {
                Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    findings.fixLabel?.let { label ->
                        Button(
                            onClick = {
                                handle.fixReviewFindings(null)
                                onDismiss()
                            },
                            modifier = Modifier.fillMaxWidth().height(50.dp).testTag("review-fix-all"),
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.onSurface,
                                contentColor = MaterialTheme.colorScheme.surface,
                            ),
                        ) { Text(label) }
                    }
                    FilledTonalButton(
                        onClick = { diff = "" },
                        modifier = Modifier.fillMaxWidth().height(50.dp).testTag("review-open-changes"),
                    ) { Text(findings.openChanges) }
                }
            }
            item { Box(Modifier.navigationBarsPadding().height(16.dp)) }
        }
    }
    diff?.let { focus ->
        FindingsDiffSheet(extras, findings.changesTitle, focus.ifEmpty { null }) {
            diff = null
            local++
        }
    }
}

@Composable
fun ChangesSheet(extras: SessionExtras, title: String, onDismiss: () -> Unit) {
    FindingsDiffSheet(extras, title, null, onDismiss)
}

@Composable
private fun FindingsDiffSheet(extras: SessionExtras, title: String, focus: String?, onDismiss: () -> Unit) {
    val handle = extras.handle
    DiffSheet(
        title = title,
        subtitle = null,
        onDismiss = onDismiss,
        focus = focus,
        finding = { card, action ->
            when (action) {
                FindingAction.Fix -> handle.fixReviewFindings(card.id)
                FindingAction.Dismiss -> handle.dismissReviewFinding(card.id)
            }
        },
    ) { handle.reviewChanges() }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun FindingRow(row: ReviewFindingRow, index: Int, count: Int, onOpen: () -> Unit, onDismiss: () -> Unit) {
    val (sevInk, sevWash) = severityColors(row.severity)
    val (stateInk, stateFill, stateBorder) = stateColors(row.state)
    SegmentedListItem(
        onClick = onOpen,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        trailingContent = if (row.dismissable) {
            {
                IconButton(onClick = onDismiss, modifier = Modifier.testTag("review-dismiss-${row.id}")) {
                    ZIcon(ZIcons.Close, "${row.dismissLabel} ${row.title}", Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        } else {
            null
        },
        modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap).testTag("review-finding-${row.id}"),
    ) {
        Row(Modifier.alpha(if (row.dimmed) 0.5f else 1f), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Tag(row.severityWord, sevInk, sevWash, null, FontWeight.SemiBold)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(row.title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
                row.reason?.let {
                    Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis)
                }
                Row(Modifier.padding(top = 3.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Tag(row.stateLabel, stateInk, stateFill, stateBorder, FontWeight.Medium)
                    Text(
                        row.meta,
                        fontFamily = GeistMono,
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.outline,
                        maxLines = 1,
                        overflow = TextOverflow.MiddleEllipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun Tag(text: String, ink: Color, fill: Color?, border: Color?, weight: FontWeight) {
    val shape = RoundedCornerShape(5.dp)
    var modifier = Modifier.background(fill ?: Color.Transparent, shape)
    if (border != null) modifier = modifier.border(1.dp, border, shape)
    Text(text, modifier.padding(horizontal = 6.dp, vertical = 2.dp), color = ink, fontSize = 11.sp, fontWeight = weight, maxLines = 1)
}

@Composable
private fun severityColors(severity: ReviewSeverity): Pair<Color, Color> {
    val danger = MaterialTheme.colorScheme.error
    return when (severity) {
        ReviewSeverity.BUG -> danger to danger.copy(alpha = 0.14f)
        ReviewSeverity.RISK -> danger.copy(alpha = 0.6f) to danger.copy(alpha = 0.08f)
        ReviewSeverity.NIT -> MaterialTheme.colorScheme.onSurfaceVariant to MaterialTheme.colorScheme.surfaceContainerHighest
    }
}

@Composable
private fun stateColors(state: ReviewFindingState): Triple<Color, Color?, Color?> {
    val palette = rememberDiffPalette()
    val success = palette.color(ColorRole.SUCCESS)
    val warning = palette.color(ColorRole.WARNING)
    val accent = MaterialTheme.colorScheme.primary
    val secondary = MaterialTheme.colorScheme.onSurfaceVariant
    return when (state) {
        ReviewFindingState.FIXED -> Triple(success, success.copy(alpha = 0.12f), null)
        ReviewFindingState.SKIPPED -> Triple(warning, warning.copy(alpha = 0.12f), null)
        ReviewFindingState.NO_CHANGE -> Triple(secondary, MaterialTheme.colorScheme.surfaceContainerHighest, null)
        ReviewFindingState.OPEN -> Triple(secondary, null, MaterialTheme.colorScheme.outline)
        ReviewFindingState.AGENT_ON_IT -> Triple(accent, null, accent.copy(alpha = 0.6f))
    }
}
