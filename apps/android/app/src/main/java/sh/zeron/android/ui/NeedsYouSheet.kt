package sh.zeron.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.composer.askColor
import sh.zeron.android.ui.composer.rememberOpenInputs
import sh.zeron.android.ui.composer.rememberWaiting
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.UserInputAnswer

private const val WAITING = "Waiting for you"
private const val QUESTION = "Question"

@Composable
fun NeedsYouPill(model: AppModel, chatId: String, onClick: () -> Unit) {
    val waiting = rememberWaiting(model, chatId)
    var shown by remember { mutableStateOf(waiting) }
    if (waiting.isNotEmpty()) shown = waiting
    AnimatedVisibility(
        waiting.isNotEmpty(),
        enter = fadeIn() + scaleIn(initialScale = 0.92f) + slideInVertically { it / 3 },
        exit = fadeOut() + scaleOut(targetScale = 0.92f) + slideOutVertically { it / 3 },
    ) {
        val first = shown.firstOrNull() ?: return@AnimatedVisibility
        val ask = askColor()
        val more = shown.size - 1
        Surface(
            onClick = onClick,
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.96f),
            border = BorderStroke(1.dp, ask),
            shadowElevation = 3.dp,
            modifier = Modifier.height(32.dp).testTag("needs-you-pill").semantics {
                contentDescription = "${first.title}, $WAITING" + if (more > 0) ", $more more waiting" else ""
            },
        ) {
            Row(Modifier.padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ZIcon(ZIcons.Info, null, Modifier.size(17.dp), tint = ask)
                Text(
                    first.title,
                    style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.SemiBold),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                Text("· $WAITING", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                if (more > 0) {
                    Box(Modifier.height(18.dp).clip(RoundedCornerShape(9.dp)).background(ask.copy(alpha = 0.14f)).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
                        Text("+$more", style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold), color = ask)
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun NeedsYouSheet(model: AppModel, excluding: String?, onDismiss: () -> Unit, onOpenChat: (String) -> Unit) {
    val sheet = rememberModalBottomSheetState()
    val scope = rememberCoroutineScope()
    var answered by remember { mutableStateOf(mapOf<String, String>()) }
    val all = rememberWaiting(model, excluding)
    val inputs = rememberOpenInputs(model, all)
    val waiting = remember(all, inputs, answered) {
        all.filter { row -> answered[row.id]?.let { request -> inputs[row.id]?.requestId.let { it == null || it == request } } != true }
    }
    var asking by remember { mutableStateOf<String?>(null) }
    var seen by remember { mutableStateOf(false) }
    if (waiting.isNotEmpty()) seen = true

    fun hideThen(run: () -> Unit) {
        scope.launch { sheet.hide() }.invokeOnCompletion { run() }
    }
    fun open(row: SessionRow) {
        if (inputs[row.id]?.questions.isNullOrEmpty()) hideThen { onOpenChat(row.id) } else {
            asking = row.id
            scope.launch { sheet.expand() }
        }
    }
    fun answer(row: SessionRow, input: InputRequest, answers: List<UserInputAnswer>) {
        val ok = runCatching {
            val client = model.client.value ?: error("signed out")
            (client.session(row.id) ?: client.openSession(row.id)).respondInput(input.requestId, answers)
        }.isSuccess
        if (!ok) return model.toast("Couldn't send. Try again.")
        model.noteDeliveryActivity()
        answered = answered + (row.id to input.requestId)
        asking = null
        model.toast("Sent to ${row.title}")
    }

    LaunchedEffect(seen, waiting.isEmpty()) {
        if (seen && waiting.isEmpty()) hideThen(onDismiss)
    }
    BackHandler(asking != null) { asking = null }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        val row = asking?.let { id -> waiting.firstOrNull { it.id == id } }
        val input = row?.let { inputs[it.id] }
        LaunchedEffect(asking, input == null) { if (asking != null && input == null) asking = null }
        AnimatedContent(row != null && input != null, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "needs-you") { question ->
            Column(Modifier.fillMaxWidth().imePadding()) {
                if (question && row != null && input != null) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        TonalCircleButton(ZIcons.Back, "Needs you", onClick = { asking = null }, size = 40.dp, modifier = Modifier.testTag("needs-you-back"))
                        Spacer(Modifier.width(12.dp))
                        Text(row.title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    }
                    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
                        Text(
                            listOfNotNull(QUESTION, row.deviceName).joinToString(" · "),
                            style = MaterialTheme.typography.labelLarge,
                            color = askColor(),
                            modifier = Modifier.padding(horizontal = 24.dp, vertical = 4.dp),
                        )
                        Spacer(Modifier.height(6.dp))
                        if (input.destructive) {
                            for (q in input.questions) {
                                Column(Modifier.padding(horizontal = 24.dp, vertical = 6.dp)) {
                                    if (q.header.isNotEmpty()) Text(q.header, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Text(q.question, style = MaterialTheme.typography.titleMedium)
                                }
                            }
                            Text(
                                "This changes files or the remote. Review it in the thread.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                            )
                        } else {
                            QuestionPanel(input) { answer(row, input, it) }
                        }
                        TextButton(onClick = { hideThen { onOpenChat(row.id) } }, modifier = Modifier.padding(horizontal = 12.dp).testTag("needs-you-open-thread")) { Text("Open thread") }
                    }
                } else {
                    WaitingList(waiting, inputs, ::open)
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun WaitingList(waiting: List<SessionRow>, inputs: Map<String, InputRequest>, onOpen: (SessionRow) -> Unit) {
    Text(
        "Needs you · ${waiting.size}",
        style = MaterialTheme.typography.titleLarge,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 8.dp),
    )
    LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 32.dp), modifier = Modifier.fillMaxWidth().testTag("needs-you-list")) {
        if (waiting.isEmpty()) {
            item("empty") {
                Text(
                    "Nothing needs you",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 20.dp),
                )
            }
        }
        itemsIndexed(waiting, key = { _, row -> row.id }) { i, row ->
            val asks = !inputs[row.id]?.questions.isNullOrEmpty()
            SegmentedListItem(
                onClick = { onOpen(row) },
                shapes = segmentedShapes(i, waiting.size),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).testTag("needs-you-${row.id}"),
                leadingContent = { ZIcon(ZIcons.Info, null, Modifier.size(22.dp), tint = askColor()) },
                supportingContent = {
                    Text(listOfNotNull(row.deviceName, if (asks) QUESTION else WAITING, row.project?.name, row.timeLabel).joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                },
                trailingContent = { Text(if (asks) "Answer" else "Open", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary) },
            ) {
                Text(row.title, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}
