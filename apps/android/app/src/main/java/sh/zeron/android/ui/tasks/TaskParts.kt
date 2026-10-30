package sh.zeron.android.ui.tasks

import androidx.annotation.DrawableRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.TonalCircleButton
import uniffi.zeron_core.StepStatus
import uniffi.zeron_core.TaskStatus
import uniffi.zeron_core.TreeStatus

sealed interface Glyph {
    data class Dot(val color: Color) : Glyph
    data object Spinner : Glyph
    data class Check(val color: Color) : Glyph
}

object Tones {
    @Composable private fun pick(light: Long, dark: Long, alpha: Float): Color =
        Color(if (LocalDarkTheme.current) dark or 0xFF000000 else light or 0xFF000000).copy(alpha = alpha)

    val working @Composable get() = pick(0x5B43E8, 0x8B7CF6, 0.55f)
    val input @Composable get() = pick(0x5B43E8, 0x8B7CF6, 0.6f)
    val failed @Composable get() = pick(0xDC2626, 0xF87171, 0.65f)
    val done @Composable get() = pick(0x15803D, 0x34D399, 0.9f)
    val idle @Composable get() = if (LocalDarkTheme.current) Color.White.copy(alpha = 0.14f) else Color.Black.copy(alpha = 0.14f)
    val warning @Composable get() = pick(0xA16207, 0xFACC15, 1f)
    val tertiary @Composable get() = pick(0x97979F, 0x6B6B72, 1f)
}

@Composable
fun glyph(status: TaskStatus): Glyph = when (status) {
    TaskStatus.TO_DO -> Glyph.Dot(Tones.idle)
    TaskStatus.WORKING -> Glyph.Spinner
    TaskStatus.NEEDS_YOU -> Glyph.Dot(Tones.input)
    TaskStatus.DONE -> Glyph.Check(Tones.done)
}

@Composable
fun glyph(status: StepStatus): Glyph = when (status) {
    StepStatus.PLANNED -> Glyph.Dot(Tones.idle)
    StepStatus.WORKING -> Glyph.Spinner
    StepStatus.NEEDS_YOU -> Glyph.Dot(Tones.warning)
    StepStatus.DONE -> Glyph.Check(Tones.done)
    StepStatus.SKIPPED -> Glyph.Dot(Tones.tertiary)
    StepStatus.FAILED -> Glyph.Dot(Tones.failed)
}

@Composable
fun glyph(status: TreeStatus): Glyph = when (status) {
    TreeStatus.RUNNING -> Glyph.Spinner
    TreeStatus.WAITING -> Glyph.Dot(Tones.warning)
    TreeStatus.DONE -> Glyph.Check(Tones.done)
    TreeStatus.FAILED -> Glyph.Dot(Tones.failed)
    TreeStatus.QUEUED, TreeStatus.STOPPED -> Glyph.Dot(Tones.idle)
}

fun stepWord(status: StepStatus): String = when (status) {
    StepStatus.PLANNED -> "Planned"
    StepStatus.WORKING -> "Working"
    StepStatus.NEEDS_YOU -> "Needs you"
    StepStatus.DONE -> "Done"
    StepStatus.SKIPPED -> "Skipped"
    StepStatus.FAILED -> "Failed"
}

@Composable
fun StatusGlyph(g: Glyph, modifier: Modifier = Modifier) {
    Box(modifier.size(12.dp), contentAlignment = Alignment.Center) {
        when (g) {
            is Glyph.Dot -> Box(Modifier.size(9.dp).clip(CircleShape).background(g.color))
            Glyph.Spinner -> CircularProgressIndicator(Modifier.size(11.dp), strokeWidth = 1.6.dp, color = Tones.working)
            is Glyph.Check -> ZIcon(ZIcons.Check, null, Modifier.size(12.dp), tint = g.color)
        }
    }
}

@Composable
fun BrandMarks(brands: List<String>) {
    if (brands.isEmpty()) return
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        for (b in brands) HarnessMark(b, 14.dp, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
fun ListRow(
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
    glyph: Glyph? = null,
    @DrawableRes icon: Int? = null,
    tint: Color = MaterialTheme.colorScheme.onSurface,
    iconTint: Color? = null,
    strong: Boolean = false,
    chevron: Boolean = false,
    indent: Dp = 0.dp,
    container: Color = Color.Transparent,
    onClick: (() -> Unit)? = null,
    trailing: (@Composable RowScope.() -> Unit)? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .background(container)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .heightIn(min = 52.dp)
            .padding(start = 16.dp + indent, end = 14.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (glyph != null) {
            StatusGlyph(glyph)
            Spacer(Modifier.width(12.dp))
        } else if (icon != null) {
            ZIcon(icon, null, Modifier.size(20.dp), tint = iconTint ?: if (tint == MaterialTheme.colorScheme.onSurface) MaterialTheme.colorScheme.onSurfaceVariant else tint)
            Spacer(Modifier.width(12.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = if (strong) FontWeight.SemiBold else FontWeight.Medium,
                color = tint,
            )
            if (!subtitle.isNullOrEmpty()) {
                Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 2.dp))
            }
        }
        trailing?.let {
            Spacer(Modifier.width(8.dp))
            it()
        }
        if (chevron) {
            Spacer(Modifier.width(6.dp))
            ZIcon(ZIcons.ChevronRight, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

fun LazyListScope.group(key: String, header: String?, content: @Composable () -> Unit) {
    item(key = key) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 18.dp)) {
            if (header != null) {
                Text(
                    header,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 16.dp, bottom = 6.dp),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceContainerLow) {
                Column { content() }
            }
        }
    }
}

@Composable
fun PageBar(title: String, onBack: (() -> Unit)?, actions: @Composable RowScope.() -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, size = 44.dp)
            Spacer(Modifier.width(12.dp))
        }
        Text(
            title,
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.weight(1f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically, content = actions)
    }
}

@Composable
fun accentSoft(): Color = MaterialTheme.colorScheme.primary.copy(alpha = 0.10f)
