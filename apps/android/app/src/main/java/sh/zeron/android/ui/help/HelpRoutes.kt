package sh.zeron.android.ui.help

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.style.TextOverflow
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.TonalCircleButton
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.first
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import sh.zeron.android.core.AppModel
import sh.zeron.android.transcript.Transcript
import sh.zeron.android.transcript.TranscriptActions
import sh.zeron.android.transcript.TranscriptState
import sh.zeron.android.ui.Chevron
import sh.zeron.android.ui.ImageViewer
import sh.zeron.android.ui.SettingsPage
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentedShapes
import sh.zeron.android.ui.settingsFooter
import sh.zeron.android.ui.settingsSection
import uniffi.zeron_core.DebugEntry
import uniffi.zeron_core.changelog
import uniffi.zeron_core.docsBody
import uniffi.zeron_core.docsImage
import uniffi.zeron_core.docsLink
import uniffi.zeron_core.docsSections
import uniffi.zeron_core.whatsNew

private fun bundledImage(name: String): ImageBitmap? =
    docsImage(name)?.let { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() }

@Composable
fun WhatsNewScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    var full by remember { mutableStateOf(false) }
    val entries = remember(full) { if (full) changelog() else listOfNotNull(whatsNew()) }
    var viewing by remember { mutableStateOf<ImageBitmap?>(null) }
    SettingsPage(if (full) "Changelog" else "What's New", if (full) ({ full = false }) else onBack) {
        entries.forEach { entry ->
            item(key = "v${entry.version}") {
                Text(
                    "v${entry.version} · ${entry.date}",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 16.dp),
                )
            }
            entry.features.forEachIndexed { i, feature ->
                item(key = "v${entry.version}/$i") {
                    Column(Modifier.padding(horizontal = 24.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(feature.title, style = MaterialTheme.typography.titleMedium)
                        Text(feature.description, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        feature.image?.let { name ->
                            val image = remember(name) { bundledImage(name) }
                            if (image != null) {
                                Spacer(Modifier.height(6.dp))
                                Image(
                                    image,
                                    feature.title,
                                    Modifier
                                        .fillMaxWidth()
                                        .aspectRatio(image.width.toFloat() / image.height)
                                        .clip(RoundedCornerShape(12.dp))
                                        .clickable { viewing = image },
                                    contentScale = ContentScale.Fit,
                                )
                            }
                        }
                    }
                }
            }
        }
        if (!full) {
            item {
                TextButton(onClick = { full = true }, modifier = Modifier.padding(start = 12.dp, top = 4.dp)) { Text("View changelog →") }
            }
        }
    }
    viewing?.let { ImageViewer(it) { viewing = null } }
    if (full) androidx.activity.compose.BackHandler { full = false }
}

@Composable
fun DocsScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    val sections = remember { docsSections() }
    SettingsPage("Docs", onBack) {
        sections.forEachIndexed { s, section ->
            settingsSection(section.title)
            item(key = section.title) {
                Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                    section.articles.forEachIndexed { i, article ->
                        SegmentedListItem(
                            onClick = { onOpenRoute("doc:${article.id}") },
                            shapes = segmentedShapes(i, section.articles.size),
                            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                            supportingContent = { Text(article.blurb) },
                            trailingContent = { Chevron() },
                        ) { Text(article.title) }
                    }
                }
            }
            if (s == sections.lastIndex) settingsFooter("Links to settings point at the Mac app; links to other articles open here.")
        }
    }
}

@Composable
fun DocScreen(model: AppModel, id: String, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    val title = remember(id) { docsSections().flatMap { it.articles }.firstOrNull { it.id == id }?.title ?: "Docs" }
    val state = remember(id) { TranscriptState().apply { following = false } }
    DisposableEffect(state) { onDispose { state.close() } }
    LaunchedEffect(state) {
        snapshotFlow { state.viewport }.first { it > 0f }
        state.engine.setDebugEntries(listOf(DebugEntry(id, false, docsBody(id).orEmpty(), false)), false)
        snapshotFlow { state.frame?.rowCount() ?: 0u }.first { it > 0u }
        state.following = false
        state.offset = 0f
    }
    var viewing by remember { mutableStateOf<ImageBitmap?>(null) }
    val uri = LocalUriHandler.current
    val actions = remember(id) {
        TranscriptActions(
            openUrl = { url ->
                val article = docsLink(url)
                when {
                    article != null -> onOpenRoute("doc:$article")
                    url.startsWith("zeron:") -> model.toast("Open this in Zeron on the Mac")
                    else -> runCatching { uri.openUri(url) }
                }
            },
            loadImage = { name -> withContext(Dispatchers.Default) { bundledImage(name) } },
            showText = { _, _, _ -> },
            openImage = { name -> viewing = bundledImage(name) },
        )
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).statusBarsPadding()) {
        Row(Modifier.padding(start = 8.dp, end = 16.dp, top = 8.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, size = 44.dp, container = MaterialTheme.colorScheme.surfaceContainerHighest)
            Spacer(Modifier.width(12.dp))
            Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Transcript(state, actions, Modifier.weight(1f).fillMaxWidth())
    }
    viewing?.let { ImageViewer(it) { viewing = null } }
}
