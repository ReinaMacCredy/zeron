package sh.zeron.android.ui.git

import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentShape
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.PrChip
import uniffi.zeron_core.PrGlyph
import uniffi.zeron_core.PrTone

@Composable
fun PrTone.color(): Color {
    val palette = rememberDiffPalette()
    return when (this) {
        PrTone.MUTED -> MaterialTheme.colorScheme.onSurfaceVariant
        PrTone.POSITIVE -> palette.color(ColorRole.SUCCESS)
        PrTone.WARNING -> palette.color(ColorRole.WARNING)
        PrTone.DANGER -> palette.color(ColorRole.DANGER)
        PrTone.MERGED -> palette.color(ColorRole.ACCENT)
    }
}

@get:DrawableRes
val PrGlyph.icon: Int
    get() = when (this) {
        PrGlyph.PULL_REQUEST -> ZIcons.PullRequest
        PrGlyph.CLOSE_CIRCLE -> ZIcons.Close
        PrGlyph.DOCUMENT -> ZIcons.File
        PrGlyph.CHECK -> ZIcons.Check
        PrGlyph.DANGER_TRIANGLE -> ZIcons.Warning
        PrGlyph.CLOCK_CIRCLE -> ZIcons.Queue
    }

@Composable
fun PrChipLabel(chip: PrChip) {
    val tone = chip.tone.color()
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        ZIcon(chip.glyph.icon, null, Modifier.size(16.dp), tint = tone)
        Text(chip.label, style = MaterialTheme.typography.labelLarge, color = tone)
    }
}

@Composable
fun changesText(text: String): AnnotatedString {
    val palette = rememberDiffPalette()
    return buildAnnotatedString {
        text.split(' ').filter { it.isNotEmpty() }.forEachIndexed { i, part ->
            if (i > 0) append(" ")
            withStyle(SpanStyle(color = palette.color(if (part.startsWith("+")) ColorRole.SUCCESS else ColorRole.DANGER), fontFamily = GeistMono, fontWeight = FontWeight.Medium, fontSize = 12.sp)) { append(part) }
        }
    }
}

class GroupRow(val key: String, val onClick: (() -> Unit)? = null, val content: @Composable () -> Unit)

@Composable
fun Group(rows: List<GroupRow>, modifier: Modifier = Modifier) {
    Column(modifier.padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        rows.forEachIndexed { i, row ->
            GroupCell(segmentShape(i, rows.size), row.onClick) { row.content() }
        }
    }
}

@Composable
fun GroupCell(shape: Shape, onClick: (() -> Unit)?, content: @Composable () -> Unit) {
    val body: @Composable () -> Unit = {
        Box(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 16.dp, vertical = 10.dp), contentAlignment = Alignment.CenterStart) { content() }
    }
    if (onClick != null) {
        Surface(onClick = onClick, shape = shape, color = cardColor(), modifier = Modifier.fillMaxWidth(), content = body)
    } else {
        Surface(shape = shape, color = cardColor(), modifier = Modifier.fillMaxWidth(), content = body)
    }
}

@Composable
fun InfoRow(label: String, onClick: Boolean = false, value: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
        Box(Modifier.padding(start = 12.dp)) { value() }
        if (onClick) ZIcon(ZIcons.ChevronRight, null, Modifier.padding(start = 4.dp).size(18.dp), tint = MaterialTheme.colorScheme.outline)
    }
}

@Composable
fun Segments(labels: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    SingleChoiceSegmentedButtonRow(modifier.fillMaxWidth()) {
        labels.forEachIndexed { i, label ->
            SegmentedButton(
                selected = i == selected,
                onClick = { onSelect(i) },
                shape = SegmentedButtonDefaults.itemShape(i, labels.size),
                icon = {},
            ) { Text(label, maxLines = 1) }
        }
    }
}

class MenuChoice(val label: String, val detail: String? = null, val checked: Boolean = false, val enabled: Boolean = true, val onClick: () -> Unit)

@Composable
fun MenuPill(
    label: String,
    choices: List<MenuChoice>,
    @DrawableRes leading: Int? = null,
    @DrawableRes trailing: Int? = ZIcons.ChevronDown,
    title: String? = null,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        Surface(onClick = { open = true }, shape = RoundedCornerShape(50), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
            Row(Modifier.heightIn(min = 36.dp).padding(horizontal = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                leading?.let { ZIcon(it, null, Modifier.size(16.dp)) }
                Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1)
                trailing?.let { ZIcon(it, null, Modifier.size(14.dp)) }
            }
        }
        ChoiceMenu(open, { open = false }, choices, title)
    }
}

@Composable
fun ChoiceMenu(open: Boolean, onDismiss: () -> Unit, choices: List<MenuChoice>, title: String? = null) {
    DropdownMenu(expanded = open, onDismissRequest = onDismiss, shape = RoundedCornerShape(16.dp)) {
        if (title != null) {
            Text(title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
        }
        for (choice in choices) {
            DropdownMenuItem(
                text = {
                    Column {
                        Text(choice.label)
                        choice.detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                },
                enabled = choice.enabled,
                trailingIcon = if (choice.checked) ({ ZIcon(ZIcons.Check, null, Modifier.size(18.dp)) }) else null,
                onClick = {
                    onDismiss()
                    choice.onClick()
                },
            )
        }
    }
}
