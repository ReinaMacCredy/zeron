package sh.zeron.android.ui.composer

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import sh.zeron.android.R
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.ActiveQuery
import sh.zeron.android.ui.ComposerModel
import sh.zeron.android.ui.composerContainer
import uniffi.zeron_core.FileMatch
import uniffi.zeron_core.InvocationCatalog
import uniffi.zeron_core.InvocationEntry
import uniffi.zeron_core.ThreadMention
import uniffi.zeron_core.filterInvocations

private sealed interface Shown {
    data object Hidden : Shown
    data object Loading : Shown
    data class Invocations(val entries: List<InvocationEntry>, val note: String?, val error: Boolean) : Shown
    data class Mentions(val threads: List<ThreadMention>, val files: List<FileMatch>) : Shown
}

@Composable
fun ComposerSuggestions(
    draft: ComposerModel,
    files: (suspend (String) -> List<FileMatch>)?,
    threads: ((String) -> List<ThreadMention>)?,
    catalog: (suspend (skill: Boolean) -> InvocationCatalog?)?,
    onInvocation: (ActiveQuery, InvocationEntry) -> Unit,
    modifier: Modifier = Modifier,
) {
    val invocation = if (catalog != null) draft.activeInvocation() else null
    val mention = if (invocation == null && (files != null || threads != null)) draft.activeQuery() else null
    var shown by remember { mutableStateOf<Shown>(Shown.Hidden) }
    val key = invocation?.let { "${if (it.second) '$' else '/'}${it.first.query}" } ?: mention?.let { "@${it.second}" }
    LaunchedEffect(key) {
        when {
            invocation != null && catalog != null -> coroutineScope {
                val (q, skill) = invocation
                val loading = async {
                    delay(150)
                    shown = Shown.Loading
                }
                val found = catalog(skill)
                loading.cancel()
                if (found == null) {
                    shown = Shown.Hidden
                    return@coroutineScope
                }
                val matches = filterInvocations(found, q.query)
                shown = Shown.Invocations(matches.entries, matches.error ?: matches.empty, matches.error != null)
            }
            mention != null -> {
                val q = mention.second
                val t = threads?.invoke(q).orEmpty()
                shown = Shown.Mentions(t, (shown as? Shown.Mentions)?.files.orEmpty())
                delay(120)
                val f = runCatching { files?.invoke(q) }.getOrNull().orEmpty()
                shown = Shown.Mentions(t, f.take(if (t.isEmpty()) 5 else 3))
            }
            else -> shown = Shown.Hidden
        }
    }
    val visible = key != null && when (val s = shown) {
        Shown.Hidden -> false
        Shown.Loading -> true
        is Shown.Invocations -> s.entries.isNotEmpty() || s.note != null
        is Shown.Mentions -> s.threads.isNotEmpty() || s.files.isNotEmpty()
    }
    AnimatedVisibility(visible, enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically(), modifier = modifier) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = composerContainer(),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
            modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp).testTag("suggestions-list"),
        ) {
            Column(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState()).padding(vertical = 4.dp)) {
                when (val s = shown) {
                    Shown.Hidden -> {}
                    Shown.Loading -> listOf(0.42f, 0.6f, 0.5f).forEach { Skeleton(it) }
                    is Shown.Invocations -> {
                        s.note?.let { Note(it, if (s.error) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant) }
                        s.entries.forEachIndexed { i, entry ->
                            InvocationRow(entry, Modifier.testTag("invocation-$i")) {
                                val q = draft.activeInvocation()?.first ?: return@InvocationRow
                                if (!entry.blocked) onInvocation(q, entry)
                            }
                        }
                    }
                    is Shown.Mentions -> {
                        if (s.threads.isNotEmpty()) {
                            Header("Threads")
                            s.threads.forEachIndexed { i, t ->
                                Row2(R.drawable.zi_chat_round_line, t.title, listOfNotNull(t.project, t.status).joinToString(" · "), mono = false, Modifier.testTag("mention-thread-$i")) { draft.insertThread(t) }
                            }
                            if (s.files.isNotEmpty()) Header("Files")
                        }
                        s.files.forEachIndexed { i, file ->
                            val path = file.path.trimEnd('/')
                            Row2(if (file.isDir) ZIcons.Folder else ZIcons.Text, path.substringAfterLast('/'), path.substringBeforeLast('/', ""), mono = true, Modifier.testTag("mention-$i")) { draft.insertMention(file) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun InvocationRow(entry: InvocationEntry, modifier: Modifier, onPick: () -> Unit) {
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 6.dp)
            .clip(RoundedCornerShape(18.dp))
            .clickable(enabled = !entry.blocked, onClick = onPick)
            .alpha(if (entry.blocked) 0.45f else if (entry.terminal) 0.55f else 1f)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ZIcon(if (entry.skill) R.drawable.zi_magic_stick_3 else ZIcons.Command, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            entry.title,
            style = MaterialTheme.typography.bodyLargeEmphasized,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = if (entry.detail.isEmpty()) Modifier else Modifier.weight(0.55f, fill = false),
        )
        if (entry.detail.isNotEmpty()) {
            Text(entry.detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
        }
        if (entry.terminal) {
            Text(
                "Terminal",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.clip(RoundedCornerShape(4.dp)).background(MaterialTheme.colorScheme.surfaceContainerHighest).padding(horizontal = 6.dp, vertical = 1.dp),
            )
        }
    }
}

@Composable
private fun Row2(icon: Int, title: String, subtitle: String, mono: Boolean, modifier: Modifier, onPick: () -> Unit) {
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 6.dp)
            .clip(RoundedCornerShape(18.dp))
            .clickable(onClick = onPick)
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        ZIcon(icon, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Column {
            Text(title, style = MaterialTheme.typography.bodyLargeEmphasized, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (subtitle.isNotEmpty()) {
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = if (mono) GeistMono else null,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun Header(text: String) {
    Text(text, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 18.dp, end = 18.dp, top = 6.dp, bottom = 2.dp))
}

@Composable
private fun Note(text: String, color: androidx.compose.ui.graphics.Color) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = color, modifier = Modifier.padding(horizontal = 18.dp, vertical = 10.dp).testTag("invocation-note"))
}

@Composable
private fun Skeleton(fraction: Float) {
    Box(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 13.dp)) {
        Box(Modifier.fillMaxWidth(fraction).height(12.dp).clip(RoundedCornerShape(5.dp)).background(MaterialTheme.colorScheme.surfaceContainerHighest))
    }
}
