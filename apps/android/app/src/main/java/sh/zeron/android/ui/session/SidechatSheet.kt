package sh.zeron.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.SectionHeader
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.chipContainer
import sh.zeron.android.ui.segmentedShapes

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SidechatSheet(extras: SessionExtras, prompt: String?, onOpenChat: (String) -> Unit, onDismiss: () -> Unit) {
    val content = remember(extras.chatId) { extras.core.sidechatSheet(extras.chatId, extras.model.row(extras.chatId)?.title ?: "") }
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }

    fun show(id: String) {
        scope.launch { sheet.hide() }.invokeOnCompletion {
            onDismiss()
            onOpenChat(id)
        }
    }

    fun run(work: suspend () -> String) {
        if (busy) return
        busy = true
        extras.launch("Couldn't open the side chat. Try again.") {
            try {
                show(work())
            } finally {
                busy = false
            }
        }
    }

    LaunchedEffect(Unit) { if (prompt != null) run { extras.core.openSidechat(extras.chatId, prompt) } }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
            Text(
                content.title,
                style = MaterialTheme.typography.titleLarge,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp),
            )
            Text(
                content.context,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 4.dp, bottom = 8.dp),
            )
            content.section?.let { SectionHeader(it) }
            Column(Modifier.padding(horizontal = 16.dp)) {
                content.rows.forEachIndexed { i, row ->
                    val expired = extras.model.row(row.chatId)?.sidechatExpired == true
                    SegmentedListItem(
                        onClick = { show(row.chatId) },
                        shapes = segmentedShapes(i, content.rows.size),
                        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).alpha(if (expired) 0.5f else 1f),
                        leadingContent = {
                            Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(chipContainer()), contentAlignment = Alignment.Center) {
                                ZIcon(ZIcons.Chat, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        },
                        supportingContent = { Text(row.timeLeft, maxLines = 1) },
                        trailingContent = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
                    ) { Text(row.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                }
            }
            Button(
                onClick = { run { extras.core.createSidechat(extras.chatId) } },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp).height(48.dp),
            ) { Text("Create") }
        }
    }
}
