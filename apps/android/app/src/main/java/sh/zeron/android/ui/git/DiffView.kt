package sh.zeron.android.ui.git

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.TranscriptPalette
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.DiffFileView
import uniffi.zeron_core.DiffFindingCard
import uniffi.zeron_core.DiffLineKind
import uniffi.zeron_core.DiffLineView
import uniffi.zeron_core.PrThread
import uniffi.zeron_core.ReviewSeverity

enum class FindingAction { Fix, Dismiss }

typealias FindingAct = (DiffFindingCard, FindingAction) -> Unit

object GitText {
    fun failure(error: Throwable, fallback: String): String = when (error) {
        is CoreException.HostException -> error.reason
        is CoreException.NotFound -> error.reason
        is CoreException.InvalidArgument -> error.reason
        is CoreException.Unsupported -> error.reason
        is CoreException.HostUnavailable -> error.reason
        else -> fallback
    }
}

@Composable
fun rememberDiffPalette(): TranscriptPalette {
    val scheme = MaterialTheme.colorScheme
    val dark = LocalDarkTheme.current
    return remember(scheme, dark) { TranscriptPalette(dark, scheme) }
}

fun TranscriptPalette.color(role: ColorRole): Color = Color(get(role))

@Composable
fun DiffDelta(additions: UInt, deletions: UInt, modifier: Modifier = Modifier) {
    val palette = rememberDiffPalette()
    val text = buildAnnotatedString {
        if (additions > 0u || deletions == 0u) withStyle(SpanStyle(color = palette.color(ColorRole.SUCCESS))) { append("+$additions") }
        if (deletions > 0u) {
            if (length > 0) append(" ")
            withStyle(SpanStyle(color = palette.color(ColorRole.DANGER))) { append("−$deletions") }
        }
    }
    Text(text, modifier, fontFamily = GeistMono, fontWeight = FontWeight.Medium, fontSize = 12.sp)
}

/**
 * The diff rows for `files` (iOS `DiffPainter`): a file header, notices, hunk
 * headers, numbered lines washed by kind, then each line's PR threads and
 * review finding cards. `focus` keys the finding card the caller scrolls to.
 */
fun LazyListScope.diffItems(
    files: List<DiffFileView>,
    finding: FindingAct? = null,
    thread: (@Composable (PrThread) -> Unit)? = null,
) {
    for (file in files) {
        item(key = "file:${file.path}") { DiffFileHeader(file) }
        file.notices.forEachIndexed { i, notice -> item(key = "notice:${file.path}:$i") { DiffNotice(notice) } }
        file.hunks.forEachIndexed { h, hunk ->
            item(key = "hunk:${file.path}:$h") { DiffHunkHeader(hunk.header) }
            hunk.lines.forEachIndexed { i, line ->
                item(key = "line:${file.path}:$h:$i") { DiffLine(line) }
                if (thread != null) line.threads.forEach { t -> item(key = "thread:${t.id}") { thread(t) } }
                line.findings.forEach { card -> item(key = "finding:${card.id}") { FindingCard(card, finding) } }
            }
        }
    }
}

/** Row index of a finding card in `diffItems(files)`, for scrolling to it. */
fun diffIndexOfFinding(files: List<DiffFileView>, id: String, withThreads: Boolean = false): Int? {
    var index = 0
    for (file in files) {
        index += 1 + file.notices.size
        for (hunk in file.hunks) {
            index += 1
            for (line in hunk.lines) {
                index += 1
                if (withThreads) index += line.threads.size
                for (card in line.findings) {
                    if (card.id == id) return index
                    index += 1
                }
            }
        }
    }
    return null
}

@Composable
fun DiffFileHeader(file: DiffFileView) {
    val name = file.path.substringAfterLast('/')
    val dir = file.path.substringBeforeLast('/', "")
    Row(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceContainerHigh).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(name, fontFamily = GeistMono, fontWeight = FontWeight.SemiBold, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                if (dir.isEmpty()) file.status else "$dir/ · ${file.status}",
                fontFamily = GeistMono,
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        DiffDelta(file.additions, file.deletions)
    }
}

@Composable
fun DiffNotice(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun DiffHunkHeader(header: String) {
    val palette = rememberDiffPalette()
    Text(
        header,
        Modifier.fillMaxWidth().background(palette.color(ColorRole.DIFF_HUNK)).padding(horizontal = 10.dp, vertical = 4.dp),
        fontFamily = GeistMono,
        fontSize = 11.sp,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

@Composable
fun DiffLine(line: DiffLineView) {
    val palette = rememberDiffPalette()
    val (wash, bar) = when (line.kind) {
        DiffLineKind.ADD -> palette.color(ColorRole.DIFF_ADD_WASH) to palette.color(ColorRole.DIFF_ADD_BAR)
        DiffLineKind.DEL -> palette.color(ColorRole.DIFF_DEL_WASH) to palette.color(ColorRole.DIFF_DEL_BAR)
        else -> Color.Transparent to Color.Transparent
    }
    val sign = when (line.kind) {
        DiffLineKind.ADD -> "+"
        DiffLineKind.DEL -> "−"
        else -> " "
    }
    val meta = line.kind == DiffLineKind.META
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min).background(wash)) {
        Box(Modifier.width(2.dp).fillMaxHeight().background(bar))
        Text(
            if (meta) "" else "${(line.oldNo?.toString() ?: "").padEnd(4)} ${line.newNo?.toString() ?: ""}",
            Modifier.width(56.dp).padding(start = 2.dp, top = 1.dp),
            fontFamily = GeistMono,
            fontSize = 11.sp,
            color = palette.color(ColorRole.TEXT_TERTIARY),
            textAlign = TextAlign.End,
            maxLines = 1,
        )
        Text(
            if (meta) line.text else "$sign ${line.text}",
            Modifier.weight(1f).padding(start = 8.dp, end = 10.dp, top = 1.dp, bottom = 1.dp),
            fontFamily = GeistMono,
            fontSize = 12.sp,
            color = if (meta) palette.color(ColorRole.TEXT_TERTIARY) else palette.color(ColorRole.CODE_TEXT),
        )
    }
}

@Composable
fun FindingCard(card: DiffFindingCard, act: FindingAct?) {
    val danger = MaterialTheme.colorScheme.error
    val ink = when (card.severity) {
        ReviewSeverity.BUG -> danger
        ReviewSeverity.RISK -> danger.copy(alpha = 0.6f)
        ReviewSeverity.NIT -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min).background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
        Box(Modifier.width(2.dp).fillMaxHeight().background(ink))
        Column(Modifier.weight(1f).padding(start = 14.dp, end = 12.dp, top = 10.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Box(Modifier.size(6.dp).clip(CircleShape).background(ink))
                Text(card.severityWord, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = ink)
                Text(card.title, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
            }
            if (card.detail.isNotEmpty()) {
                Text(card.detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (act != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    FilledTonalButton(onClick = { act(card, FindingAction.Fix) }) { Text(card.fixLabel) }
                    TextButton(onClick = { act(card, FindingAction.Dismiss) }) { Text(card.dismissLabel, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
        }
    }
}

/**
 * A diff in a bottom sheet (iOS `DiffSheetViewController`): loads with `load`,
 * reloads after a finding action, scrolls to `focus` (a finding card id) once.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DiffSheet(
    title: String,
    subtitle: String?,
    onDismiss: () -> Unit,
    focus: String? = null,
    finding: FindingAct? = null,
    load: suspend () -> List<DiffFileView>,
) {
    var files by remember { mutableStateOf<List<DiffFileView>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var generation by remember { mutableIntStateOf(0) }
    val list = rememberLazyListState()
    LaunchedEffect(generation) {
        try {
            files = load()
            error = null
        } catch (e: Exception) {
            error = GitText.failure(e, "Couldn't load the diff. Try again.")
        }
    }
    var scrolled by remember { mutableStateOf(false) }
    LaunchedEffect(files) {
        val loaded = files ?: return@LaunchedEffect
        if (scrolled || focus == null) return@LaunchedEffect
        diffIndexOfFinding(loaded, focus)?.let { list.scrollToItem(maxOf(0, it - 3)) }
        scrolled = true
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false)) {
        Column(Modifier.fillMaxSize()) {
            Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 20.dp).padding(bottom = 2.dp))
            if (subtitle != null) {
                Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 20.dp))
            }
            LazyColumn(Modifier.fillMaxWidth().weight(1f).padding(top = 8.dp), state = list) {
                val loaded = files
                when {
                    error != null -> item { DiffNotice(error!!) }
                    loaded == null -> item { DiffNotice("Loading…") }
                    loaded.isEmpty() -> item { DiffNotice("No changes to show.") }
                    else -> diffItems(
                        loaded,
                        finding = finding?.let { act ->
                            { card, action ->
                                act(card, action)
                                generation++
                            }
                        },
                    )
                }
                item { Box(Modifier.navigationBarsPadding().height(16.dp)) }
            }
        }
    }
}
