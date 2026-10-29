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
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
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
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
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
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.composer.NeedsYou
import sh.zeron.android.ui.composer.NeedsYouFilter
import sh.zeron.android.ui.composer.actionLabel
import sh.zeron.android.ui.composer.askColor
import sh.zeron.android.ui.composer.failure
import sh.zeron.android.ui.composer.filter
import sh.zeron.android.ui.composer.icon
import sh.zeron.android.ui.composer.input
import sh.zeron.android.ui.composer.isThread
import sh.zeron.android.ui.composer.itemId
import sh.zeron.android.ui.composer.kindLabel
import sh.zeron.android.ui.composer.locked
import sh.zeron.android.ui.composer.meta
import sh.zeron.android.ui.composer.rememberNeedsYou
import sh.zeron.android.ui.composer.rowTitle
import sh.zeron.android.ui.composer.sendLine
import sh.zeron.android.ui.composer.warningColor
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.ItemAnswer
import uniffi.zeron_core.ItemSend
import uniffi.zeron_core.NeedsYouEntry
import uniffi.zeron_core.NeedsYouKind
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.missionLabels

@Composable
fun NeedsYouPill(model: AppModel, chatId: String, onClick: () -> Unit) {
    val waiting = NeedsYou.waiting(rememberNeedsYou(model), chatId)
    var shown by remember { mutableStateOf(waiting) }
    if (waiting.isNotEmpty()) shown = waiting
    AnimatedVisibility(
        waiting.isNotEmpty(),
        enter = fadeIn() + scaleIn(initialScale = 0.92f) + slideInVertically { it / 3 },
        exit = fadeOut() + scaleOut(targetScale = 0.92f) + slideOutVertically { it / 3 },
    ) {
        val first = shown.firstOrNull() ?: return@AnimatedVisibility
        val ask = askColor()
        val title = model.row(first.chatId)?.takeIf { it.hasTitle }?.title ?: first.asker
        val more = shown.size - 1
        Surface(
            onClick = onClick,
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.96f),
            border = BorderStroke(1.dp, ask),
            shadowElevation = 3.dp,
            modifier = Modifier.height(32.dp).testTag("needs-you-pill").semantics {
                contentDescription = "$title, ${first.kindLabel}" + if (more > 0) ", $more more waiting" else ""
            },
        ) {
            Row(Modifier.padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ZIcon(first.icon, null, Modifier.size(17.dp), tint = ask)
                Text(
                    title,
                    style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.SemiBold),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                Text("· ${first.kindLabel}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                if (more > 0) {
                    Box(Modifier.height(18.dp).clip(RoundedCornerShape(9.dp)).background(ask.copy(alpha = 0.14f)).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
                        Text("+$more", style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold), color = ask)
                    }
                }
            }
        }
    }
}

private sealed interface Page {
    data object Home : Page
    data class Question(val key: String) : Page
    data class Group(val key: String, val title: String) : Page
    data class Plan(val key: String) : Page
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NeedsYouSheet(model: AppModel, excluding: String?, onDismiss: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    val sheet = rememberModalBottomSheetState()
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            tick++
        }
    }
    val entries = rememberNeedsYou(model, tick)
    val all = NeedsYou.all(entries, excluding)
    var stack by remember { mutableStateOf(listOf<Page>(Page.Home)) }
    var seen by remember { mutableStateOf(false) }
    if (all.isNotEmpty()) seen = true

    fun hideThen(run: () -> Unit) {
        scope.launch { sheet.hide() }.invokeOnCompletion { run() }
    }
    fun push(page: Page) {
        stack = stack + page
        scope.launch { sheet.expand() }
    }
    fun pop() {
        if (stack.size > 1) stack = stack.dropLast(1)
    }
    fun open(entry: NeedsYouEntry) {
        when (val k = entry.kind) {
            is NeedsYouKind.Input -> if (k.questions.isEmpty()) hideThen { onOpenChat(k.chatId) } else push(Page.Question(entry.key))
            is NeedsYouKind.Item -> push(Page.Group(entry.groupKey, entry.groupTitle))
            is NeedsYouKind.Plan -> push(Page.Plan(entry.key))
        }
    }
    fun answer(entry: NeedsYouEntry, answers: List<UserInputAnswer>) {
        val ask = entry.input ?: return
        val ok = runCatching {
            val client = model.client.value ?: error("signed out")
            client.openSession(ask.chatId).respondInput(ask.requestId, answers)
        }.isSuccess
        if (!ok) return model.toast("Couldn't send. Try again.")
        model.noteDeliveryActivity()
        tick++
        val left = NeedsYou.all(model.client.value?.needsYou().orEmpty(), null)
        if (left.isNotEmpty()) {
            stack = listOf(Page.Home)
            model.toast("Sent to ${entry.dest}")
        } else {
            hideThen {
                onDismiss()
                model.toast("Sent to ${entry.dest}")
            }
        }
    }

    LaunchedEffect(seen, all.isEmpty(), stack.size) {
        if (seen && all.isEmpty() && stack.size == 1) hideThen(onDismiss)
    }
    BackHandler(stack.size > 1) { pop() }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        val page = stack.last()
        AnimatedContent(page, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "needs-you") { current ->
            Column(Modifier.fillMaxWidth().imePadding()) {
                when (current) {
                    Page.Home -> HomePage(model, all, seen, ::open) { key, title -> push(Page.Group(key, title)) }
                    is Page.Question -> {
                        val entry = entries.firstOrNull { it.key == current.key }
                        LaunchedEffect(entry == null) { if (entry == null) pop() }
                        if (entry != null) {
                            PageHeader(model.row(entry.chatId)?.takeIf { it.hasTitle }?.title ?: entry.asker, ::pop)
                            QuestionPage(entry, onOpenThread = { hideThen { onOpenChat(entry.chatId) } }) { answer(entry, it) }
                        }
                    }
                    is Page.Group -> {
                        val group = entries.filter { it.groupKey == current.key && !it.isThread }
                        LaunchedEffect(group.isEmpty()) { if (group.isEmpty()) pop() }
                        PageHeader(current.title, ::pop)
                        GroupPage(model, current.key, group, onOpen = ::open, onSent = { tick++ })
                    }
                    is Page.Plan -> {
                        val entry = entries.firstOrNull { it.key == current.key }
                        LaunchedEffect(entry == null) { if (entry == null) pop() }
                        if (entry != null) {
                            PageHeader(entry.taskTitle ?: entry.groupTitle, ::pop)
                            PlanPage(
                                model,
                                entry,
                                onOpenMission = { taskId -> hideThen { onOpenRoute("tasks:mission:$taskId") } },
                                onApproved = {
                                    tick++
                                    stack = listOf(Page.Home)
                                },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PageHeader(title: String, onBack: (() -> Unit)?) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (onBack != null) {
            TonalCircleButton(ZIcons.Back, "Needs you", onClick = onBack, size = 40.dp, modifier = Modifier.testTag("needs-you-back"))
            Spacer(Modifier.width(12.dp))
        }
        Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(start = if (onBack == null) 8.dp else 0.dp))
    }
}

private data class EntryGroup(val key: String, val title: String, val entries: List<NeedsYouEntry>)

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun HomePage(model: AppModel, all: List<NeedsYouEntry>, seen: Boolean, onOpen: (NeedsYouEntry) -> Unit, onOpenGroup: (String, String) -> Unit) {
    var filter by remember { mutableStateOf(NeedsYouFilter.All) }
    val groups = remember(all, filter) {
        val next = mutableListOf<EntryGroup>()
        for (entry in all) {
            if (filter != NeedsYouFilter.All && entry.filter != filter) continue
            val key = if (entry.isThread) "threads" else entry.groupKey
            val at = next.indexOfFirst { it.key == key }
            if (at >= 0) next[at] = next[at].copy(entries = next[at].entries + entry)
            else next.add(EntryGroup(key, if (entry.isThread) "Threads" else entry.groupTitle, listOf(entry)))
        }
        val threads = next.indexOfFirst { it.key == "threads" }
        if (threads > 0) next.add(0, next.removeAt(threads))
        next
    }
    PageHeader("Needs you · ${all.size}", null)
    LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 32.dp), modifier = Modifier.fillMaxWidth().testTag("needs-you-list")) {
        item("filters") {
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (f in NeedsYouFilter.entries) {
                    val count = if (f == NeedsYouFilter.All) all.size else all.count { it.filter == f }
                    Pill(f.label, filter == f, onClick = { filter = f }, count = count)
                }
            }
        }
        if (groups.isEmpty()) {
            item("empty") {
                Text(
                    if (!seen && all.isEmpty()) "Nothing needs you" else "Nothing here",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 20.dp),
                )
            }
        }
        for (group in groups) {
            item("h-${group.key}") {
                Row(Modifier.fillMaxWidth().padding(start = 12.dp, top = 16.dp, bottom = 4.dp).heightIn(min = 36.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(group.title, style = MaterialTheme.typography.titleSmallEmphasized, color = MaterialTheme.colorScheme.primary, modifier = Modifier.weight(1f))
                    val items = group.entries.count { it.itemId != null }
                    if (items > 0) {
                        TextButton(onClick = { onOpenGroup(group.key, group.title) }, modifier = Modifier.testTag("needs-you-group-${group.key}")) {
                            Text("$items to answer", color = warningColor())
                            Spacer(Modifier.width(4.dp))
                            ZIcon(ZIcons.ChevronRight, null, Modifier.size(14.dp), tint = warningColor())
                        }
                    }
                }
            }
            itemsIndexed(group.entries, key = { _, e -> "e/${group.key}/${e.key}" }) { i, entry ->
                EntryRow(model, entry, i, group.entries.size) { onOpen(entry) }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun EntryRow(model: AppModel, entry: NeedsYouEntry, index: Int, count: Int, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    SegmentedListItem(
        onClick = onClick,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).testTag("needs-you-${entry.key}"),
        leadingContent = { ZIcon(entry.icon, null, Modifier.size(22.dp), tint = if (entry.isThread) askColor() else warningColor()) },
        supportingContent = {
            Column {
                Text("${entry.host.name} · ${entry.meta(model)}", style = MaterialTheme.typography.bodySmall)
                entry.sendLine?.let {
                    Text(it, style = MaterialTheme.typography.labelMedium, color = if (entry.failure == null) scheme.onSurfaceVariant else scheme.error)
                }
            }
        },
        trailingContent = { Text(entry.actionLabel, style = MaterialTheme.typography.labelLarge, color = scheme.primary) },
    ) {
        Text(entry.rowTitle, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun QuestionPage(entry: NeedsYouEntry, onOpenThread: () -> Unit, onSubmit: (List<UserInputAnswer>) -> Unit) {
    val ask = entry.input ?: return
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Text(
            "${entry.kindLabel} · ${entry.asker} · ${entry.host.name}",
            style = MaterialTheme.typography.labelLarge,
            color = if (entry.isThread) askColor() else warningColor(),
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 4.dp),
        )
        Spacer(Modifier.height(6.dp))
        if (ask.destructive) {
            for (q in ask.questions) {
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
            QuestionPanel(InputRequest(entry.key, ask.requestId, ask.questions), onSubmit)
        }
        TextButton(onClick = onOpenThread, modifier = Modifier.padding(horizontal = 12.dp).testTag("needs-you-open-thread")) { Text("Open thread") }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun GroupPage(model: AppModel, key: String, group: List<NeedsYouEntry>, onOpen: (NeedsYouEntry) -> Unit, onSent: () -> Unit) {
    val scope = rememberCoroutineScope()
    val picks = remember(key) { mutableStateMapOf<String, String>() }
    var sending by remember(key) { mutableStateOf(false) }
    val items = group.filter { it.itemId != null && !it.locked }
    LaunchedEffect(group) {
        picks.keys.retainAll(items.map { it.key }.toSet())
        for (entry in group) {
            val failed = entry.send as? ItemSend.Failed ?: continue
            if (entry.key !in picks) picks[entry.key] = failed.choice
        }
    }
    val dest = items.firstOrNull()?.dest ?: "the Lead"
    fun send() {
        val picked = items.mapNotNull { e -> picks[e.key]?.let { e.host.deviceId to ItemAnswer(e.itemId!!, it) } }
        val client = model.client.value ?: return
        if (picked.isEmpty()) return
        sending = true
        scope.launch {
            var failed = false
            for ((device, answers) in picked.groupBy({ it.first }, { it.second })) {
                try {
                    client.resolveSupervisorItems(device, answers)
                } catch (e: Exception) {
                    failed = true
                }
            }
            sending = false
            picks.clear()
            onSent()
            if (!failed) model.toast("Sent to $dest")
        }
    }
    Column(Modifier.fillMaxWidth()) {
        LazyColumn(contentPadding = PaddingValues(horizontal = 16.dp), modifier = Modifier.weight(1f, fill = false).fillMaxWidth().testTag("needs-you-group")) {
            for (entry in group) groupSection(model, entry, picks, onOpen)
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("${picks.size} of ${items.size} picked", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            Button(onClick = ::send, enabled = picks.isNotEmpty() && !sending, shapes = ButtonDefaults.shapes(), modifier = Modifier.testTag("needs-you-send")) {
                Text(if (picks.size > 1) "Send ${picks.size} to $dest" else "Send to $dest")
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
private fun LazyListScope.groupSection(model: AppModel, entry: NeedsYouEntry, picks: MutableMap<String, String>, onOpen: (NeedsYouEntry) -> Unit) {
    item("h/${entry.key}") {
        Text(
            "${entry.meta(model)} · ${entry.host.name}",
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 12.dp, end = 12.dp, top = 16.dp, bottom = 6.dp),
        )
    }
    val kind = entry.kind
    when {
        kind is NeedsYouKind.Item && entry.locked -> item("r/${entry.key}") {
            SegmentedListItem(
                onClick = {},
                enabled = false,
                shapes = segmentedShapes(0, 1),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor(), disabledContainerColor = cardColor()),
                supportingContent = { entry.sendLine?.let { Text(it) } },
                modifier = Modifier.testTag("needs-you-${entry.key}"),
            ) { Text(entry.title, style = MaterialTheme.typography.titleMedium) }
        }
        kind is NeedsYouKind.Item -> itemsIndexed(kind.options, key = { i, _ -> "o/${entry.key}/$i" }) { i, option ->
            val picked = picks[entry.key] == option
            SegmentedListItem(
                onClick = { if (picked) picks.remove(entry.key) else picks[entry.key] = option },
                shapes = segmentedShapes(i, kind.options.size),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).testTag("needs-you-option-${entry.key}-$i"),
                trailingContent = if (picked) {
                    { ZIcon(ZIcons.Check, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary) }
                } else {
                    null
                },
            ) {
                Column {
                    if (i == 0) Text(entry.title, style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold))
                    Text(option, style = MaterialTheme.typography.bodyLarge)
                }
            }
        }
        else -> item("r/${entry.key}") {
            val questions = entry.input?.questions.orEmpty()
            SegmentedListItem(
                onClick = { onOpen(entry) },
                shapes = segmentedShapes(0, 1),
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                supportingContent = {
                    Text(
                        when {
                            questions.any { it.secret } -> "Enter it on your Mac"
                            questions.all { it.options.isNotEmpty() } -> "Answer"
                            else -> "Typed answer · Reply in conversation"
                        },
                        color = MaterialTheme.colorScheme.primary,
                    )
                },
                trailingContent = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp)) },
                modifier = Modifier.testTag("needs-you-${entry.key}"),
            ) { Text(entry.rowTitle, style = MaterialTheme.typography.titleMedium) }
        }
    }
    val footer = entry.sendLine ?: (kind as? NeedsYouKind.Item)?.detail?.takeIf { it.isNotEmpty() }
    if (footer != null) {
        item("f/${entry.key}") {
            Text(
                footer,
                style = MaterialTheme.typography.bodySmall,
                color = if (entry.failure == null) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
            )
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun PlanPage(model: AppModel, entry: NeedsYouEntry, onOpenMission: (String) -> Unit, onApproved: () -> Unit) {
    val plan = entry.kind as? NeedsYouKind.Plan ?: return
    val scope = rememberCoroutineScope()
    val openLabel = remember { missionLabels().openMissionPage }
    var approving by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 4.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(entry.host.name, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(entry.title, style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold), color = warningColor())
        val lines = plan.steps.take(4).mapIndexed { i, step -> "Step ${i + 1} · $step" } +
            if (plan.steps.size > 4) listOf("+${plan.steps.size - 4} more") else emptyList()
        Text(lines.joinToString("\n"), style = MaterialTheme.typography.bodyLarge)
        Spacer(Modifier.height(8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            FilledTonalButton(onClick = { onOpenMission(plan.taskId) }, shapes = ButtonDefaults.shapes(), modifier = Modifier.weight(1f).testTag("needs-you-open-mission")) {
                Text(openLabel, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Button(
                onClick = {
                    val client = model.client.value ?: return@Button
                    approving = true
                    scope.launch {
                        try {
                            client.approvePlan(plan.taskId)
                            onApproved()
                            model.toast("Approved. The lead starts the steps.")
                        } catch (e: Exception) {
                            model.toast(e.message ?: "Couldn't approve the plan.")
                        } finally {
                            approving = false
                        }
                    }
                },
                enabled = !approving,
                shapes = ButtonDefaults.shapes(),
                modifier = Modifier.weight(1f).testTag("needs-you-approve-plan"),
            ) { Text("Approve and run", maxLines = 1) }
        }
    }
}
