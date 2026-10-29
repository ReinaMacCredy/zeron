package sh.zeron.android.ui.tasks

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.TasksText
import sh.zeron.android.design.ZIcons
import sh.zeron.android.transcript.Transcript
import sh.zeron.android.transcript.TranscriptActions
import sh.zeron.android.transcript.TranscriptState
import sh.zeron.android.ui.TonalCircleButton
import uniffi.zeron_core.BriefMessage
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.DebugEntry
import uniffi.zeron_core.HostRef
import uniffi.zeron_core.PlanEdit
import uniffi.zeron_core.PlanEstimate
import uniffi.zeron_core.PlanPhase
import uniffi.zeron_core.PlanStepEdit
import uniffi.zeron_core.StepStatus
import uniffi.zeron_core.StepView
import uniffi.zeron_core.TreeNodeKind
import uniffi.zeron_core.TreeStatus
import uniffi.zeron_core.missionLabels
import uniffi.zeron_core.treeLabels

private data class StepText(val title: String, val notes: String)

private data class PlanDraft(
    val edited: Map<String, StepText> = emptyMap(),
    val added: List<StepText> = emptyList(),
    val removed: Set<String> = emptySet(),
    val order: List<String>? = null,
) {
    val count: Int get() = edited.size + added.size + removed.size + if (order == null) 0 else 1

    fun shown(steps: List<StepView>): List<StepView> {
        val ordered = order?.let { o -> o.mapNotNull { id -> steps.firstOrNull { it.id == id } } + steps.filter { it.id !in o } } ?: steps
        return ordered.filter { it.id !in removed }.map { s -> edited[s.id]?.let { s.copy(title = it.title, notes = it.notes) } ?: s }
    }

    fun edit(steps: List<StepView>): PlanEdit = PlanEdit(
        overview = null,
        steps = steps.mapNotNull { s -> edited[s.id]?.let { PlanStepEdit(s.id, it.title, it.notes, emptyList()) } } +
            added.map { PlanStepEdit(null, it.title, it.notes, emptyList()) },
        removeStepIds = steps.map { it.id }.filter { it in removed },
        order = order.orEmpty().filter { it !in removed },
    )
}

private sealed interface Sub {
    data class Brief(val stepId: String?) : Sub
    data class Authority(val host: HostRef) : Sub
}

@Composable
fun MissionControl(model: AppModel, taskId: String, onBack: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    val client by model.client.collectAsState()
    val ws by model.workspace.collectAsState()
    var tick by remember { mutableIntStateOf(0) }
    val detail = remember(client, ws, tick) { client?.taskDetail(taskId) }
    var seen by remember { mutableStateOf(false) }
    LaunchedEffect(detail) { if (detail != null) seen = true else if (seen) onBack() }
    val held = detail?.approval?.held == true
    LaunchedEffect(held) {
        while (held) {
            delay(1000)
            tick++
        }
    }
    var estimate by remember { mutableStateOf<PlanEstimate?>(null) }
    val estimateKey = detail?.steps?.joinToString(",") { "${it.id}=${it.status}" }
    LaunchedEffect(estimateKey) {
        val c = client ?: return@LaunchedEffect
        if (estimateKey == null) return@LaunchedEffect
        estimate = runCatching { c.planEstimate(taskId) }.getOrNull()
    }
    var editing by remember { mutableStateOf(false) }
    var draft by remember { mutableStateOf(PlanDraft()) }
    var outline by remember { mutableStateOf(false) }
    var sub by remember { mutableStateOf<Sub?>(null) }
    var stepMenu by remember { mutableStateOf<StepView?>(null) }
    var editText by remember { mutableStateOf<Triple<String, StepText?, (StepText) -> Unit>?>(null) }
    var menu by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    fun run(action: suspend (CoreClient) -> Unit) {
        val c = client ?: return
        scope.launch { runCatching { action(c) }.onFailure { model.toast(TasksText.error(it)) } }
    }
    val labels = remember { missionLabels() }

    when (val s = sub) {
        is Sub.Brief -> {
            BackHandler { sub = null }
            BriefPage(model, taskId, s.stepId, onBack = { sub = null }, onOpenChat = onOpenChat)
            return
        }
        is Sub.Authority -> {
            BackHandler { sub = null }
            AuthorityPage(model, s.host, onBack = { sub = null })
            return
        }
        null -> Unit
    }
    val d = detail
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar(labels.missionPage, onBack) {
            if (d != null && (editing || d.steps.any { it.status == StepStatus.PLANNED })) {
                TextButton(onClick = { editing = !editing }) { Text(if (editing) "Done" else "Edit") }
            }
            Box {
                TonalCircleButton(ZIcons.More, "More", onClick = { menu = true }, size = 44.dp)
                DropdownMenu(menu, onDismissRequest = { menu = false }) {
                    val watch = d?.watch
                    val lead = d?.row?.leadChatId
                    if (watch != null && lead != null) {
                        DropdownMenuItem(
                            text = { Text("Watched by the Supervisor") },
                            trailingIcon = if (watch.watched) ({ sh.zeron.android.design.ZIcon(ZIcons.Check, null) }) else null,
                            onClick = {
                                menu = false
                                run { it.setWatched(lead, !watch.watched) }
                            },
                        )
                        DropdownMenuItem(text = { Text("Authority on ${watch.host.name}") }, onClick = {
                            menu = false
                            sub = Sub.Authority(watch.host)
                        })
                    }
                    DropdownMenuItem(text = { Text("Open Supervisor") }, onClick = {
                        menu = false
                        onOpenRoute("tasks:supervisor")
                    })
                    if (lead != null) {
                        DropdownMenuItem(text = { Text("Lead brief") }, onClick = {
                            menu = false
                            sub = Sub.Brief(null)
                        })
                    }
                }
            }
        }
        if (d == null) return@Column
        val task = d.row
        val lead = task.leadChatId
        if (lead != null) {
            val views = remember { treeLabels().views.take(2) }
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
                views.forEachIndexed { i, label ->
                    SegmentedButton(selected = (i == 1) == outline, onClick = { outline = i == 1 }, shape = SegmentedButtonDefaults.itemShape(i, views.size), icon = {}) { Text(label) }
                }
            }
        }
        if (outline && lead != null) {
            TreeOutlineList(model, lead, task.title, onOpenChat)
            return@Column
        }
        val steps = draft.shown(d.steps)
        val changes = if (draft.count == 1) "1 change" else "${draft.count} changes"
        val accent = MaterialTheme.colorScheme.primary
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
            group("header", null) {
                val header = listOfNotNull("Mission", task.host?.name, task.missionLabel ?: task.word, "${task.progressDone}/${task.progressTotal} done")
                ListRow(task.title, header.joinToString(" · "), glyph = glyph(task.status))
            }
            d.approval?.let { approval ->
                group("approval", null) {
                    ListRow(approval.line, icon = ZIcons.Check)
                    if (approval.canTakeBack) {
                        RowDivider()
                        ListRow("Take back", icon = ZIcons.Restart, tint = accent, strong = true, container = accentSoft(), onClick = { run { it.takeBackPlanApproval(taskId) } })
                    }
                }
            }
            val ready = d.phase == PlanPhase.PlanReady
            if (ready || d.plan.isNotEmpty()) {
                group("plan", if (ready) "Plan review" else "Plan") {
                    var any = false
                    if (ready) {
                        ListRow("Plan ready · ${d.steps.size} step${if (d.steps.size == 1) "" else "s"} · the lead waits for you", icon = ZIcons.File)
                        any = true
                    }
                    estimate?.takeIf { it.lines.isNotEmpty() }?.let { e ->
                        if (any) RowDivider()
                        ListRow(
                            "${e.cost} ${e.baseline}",
                            (listOf(e.mix.joinToString(", ")) + listOfNotNull(e.raised)).joinToString("\n"),
                            icon = ZIcons.Effort,
                            tint = if (e.raised == null) MaterialTheme.colorScheme.onSurface else Tones.warning,
                        )
                        any = true
                    }
                    if (d.plan.isNotEmpty()) {
                        if (any) RowDivider()
                        PlanMarkdown(d.plan, Modifier.padding(horizontal = 16.dp, vertical = 10.dp))
                    }
                    if (ready) {
                        RowDivider()
                        ListRow(
                            if (draft.count > 0) "Approve and run with $changes" else "Approve and run",
                            icon = ZIcons.Send,
                            tint = accent,
                            strong = true,
                            container = accentSoft(),
                            onClick = { sendDraft(taskId, d.steps, draft, approve = true, ::run) { draft = PlanDraft(); editing = false } },
                        )
                    }
                }
            }
            if (steps.isNotEmpty() || editing) {
                group("steps", "Steps · ${steps.size + draft.added.size}") {
                    steps.forEachIndexed { i, step ->
                        if (i > 0) RowDivider()
                        val line = estimate?.lines?.firstOrNull { it.stepId == step.id }
                        val sub = buildList {
                            add(stepWord(step.status))
                            step.question?.let { add(it) }
                            if (draft.edited.containsKey(step.id)) add("Edited")
                            line?.let { add(it.runsOn); it.multiplier?.let(::add) }
                        }.joinToString(" · ") + (line?.fallback?.let { "\n$it" } ?: "")
                        val editable = editing && step.status == StepStatus.PLANNED
                        ListRow(
                            "${step.label} · ${step.title}",
                            sub,
                            glyph = glyph(step.status),
                            chevron = !editing,
                            onClick = when {
                                editable -> ({ editText = Triple("Edit ${step.label}", StepText(step.title, step.notes)) { t -> draft = draft.copy(edited = draft.edited + (step.id to t)) } })
                                editing -> null
                                else -> ({ stepMenu = step })
                            },
                            trailing = {
                                step.brand?.let { BrandMarks(listOf(it)) }
                                if (editable) {
                                    val movable = steps.filter { it.status == StepStatus.PLANNED }
                                    val at = movable.indexOf(step)
                                    if (at > 0) TonalCircleButton(ZIcons.ChevronUp, "Move up", onClick = { draft = move(draft, d.steps, steps, step.id, -1) }, size = 32.dp)
                                    if (at < movable.lastIndex) TonalCircleButton(ZIcons.ChevronDown, "Move down", onClick = { draft = move(draft, d.steps, steps, step.id, 1) }, size = 32.dp)
                                    TonalCircleButton(ZIcons.Delete, "Delete", onClick = { draft = draft.copy(removed = draft.removed + step.id) }, size = 32.dp, content = MaterialTheme.colorScheme.error)
                                }
                            },
                        )
                    }
                    draft.added.forEachIndexed { ix, t ->
                        if (steps.isNotEmpty() || ix > 0) RowDivider()
                        ListRow(
                            t.title,
                            "New step",
                            icon = ZIcons.Plus,
                            onClick = { editText = Triple("Edit step", t) { n -> draft = draft.copy(added = draft.added.toMutableList().also { it[ix] = n }) } },
                            trailing = if (editing) ({
                                TonalCircleButton(ZIcons.Delete, "Delete", onClick = { draft = draft.copy(added = draft.added.filterIndexed { j, _ -> j != ix }) }, size = 32.dp, content = MaterialTheme.colorScheme.error)
                            }) else null,
                        )
                    }
                    if (editing) {
                        RowDivider()
                        ListRow("Add step", icon = ZIcons.Plus, tint = accent, strong = true, onClick = {
                            editText = Triple("New step", null) { t -> draft = draft.copy(added = draft.added + t) }
                        })
                    }
                }
            }
            if (draft.count > 0) {
                group("draft", null) {
                    ListRow("Send $changes to the lead", icon = ZIcons.Send, tint = accent, strong = true, container = accentSoft(), onClick = {
                        sendDraft(taskId, d.steps, draft, approve = false, ::run) { draft = PlanDraft(); editing = false }
                    })
                    RowDivider()
                    ListRow("Discard changes", icon = ZIcons.Close, tint = MaterialTheme.colorScheme.error, strong = true, onClick = { draft = PlanDraft() })
                }
            }
            if (lead != null) {
                group("lead", null) {
                    ListRow("Open lead thread", icon = ZIcons.Chat, tint = accent, strong = true, container = accentSoft(), onClick = { onOpenChat(lead) })
                }
            }
        }
    }
    stepMenu?.let { step ->
        val choices = buildList {
            if (step.chatId != null) add("Open thread" to false)
            add("Brief" to false)
            if (step.status == StepStatus.FAILED) add("Retry" to false)
            if (step.status == StepStatus.SKIPPED) add("Unskip" to false) else if (step.status != StepStatus.DONE) add("Skip" to true)
        }
        ChoiceDialog("${step.label} · ${step.title}", step.notes.ifEmpty { null }, choices, onDismiss = { stepMenu = null }) { choice ->
            stepMenu = null
            when (choice) {
                "Open thread" -> step.chatId?.let(onOpenChat)
                "Brief" -> sub = Sub.Brief(step.id)
                "Retry" -> run { it.retryStep(step.id) }
                "Unskip" -> run { it.setStepSkipped(step.id, false) }
                "Skip" -> run { it.setStepSkipped(step.id, true) }
            }
        }
    }
    editText?.let { (title, current, save) ->
        StepTextDialog(title, current, onDismiss = { editText = null }) {
            editText = null
            save(it)
        }
    }
}

private fun move(draft: PlanDraft, original: List<StepView>, shown: List<StepView>, id: String, by: Int): PlanDraft {
    val ids = shown.map { it.id }.toMutableList()
    val from = ids.indexOf(id)
    val to = from + by
    if (from < 0 || to !in ids.indices) return draft
    ids.add(to, ids.removeAt(from))
    val order = ids + original.map { it.id }.filter { it in draft.removed }
    return draft.copy(order = if (order == original.map { it.id }) null else order)
}

private fun sendDraft(taskId: String, steps: List<StepView>, draft: PlanDraft, approve: Boolean, run: (suspend (CoreClient) -> Unit) -> Unit, done: () -> Unit) {
    val edit = if (draft.count > 0) draft.edit(steps) else null
    run { c ->
        c.editTaskPlan(taskId, edit, approve)
        done()
    }
}

@Composable
private fun StepTextDialog(title: String, current: StepText?, onDismiss: () -> Unit, onSave: (StepText) -> Unit) {
    var name by remember { mutableStateOf(current?.title.orEmpty()) }
    var notes by remember { mutableStateOf(current?.notes.orEmpty()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                OutlinedTextField(name, { name = it }, placeholder = { Text("Title") }, singleLine = true)
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(notes, { notes = it }, placeholder = { Text("Notes") })
            }
        },
        confirmButton = { TextButton(enabled = name.isNotBlank(), onClick = { onSave(StepText(name, notes)) }) { Text("Save") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
fun PlanMarkdown(markdown: String, modifier: Modifier = Modifier) {
    val state = remember { TranscriptState().apply { bottomInset = 0f } }
    DisposableEffect(state) { onDispose { state.close() } }
    LaunchedEffect(markdown) {
        state.engine.setDebugEntries(listOf(DebugEntry("plan", false, markdown, false)), false)
    }
    val uri = LocalUriHandler.current
    val actions = remember {
        TranscriptActions(openUrl = { runCatching { uri.openUri(it) } }, loadImage = { null }, showText = { _, _, _ -> })
    }
    val height = state.frame?.totalHeight() ?: 0f
    Transcript(state, actions, modifier.fillMaxWidth().height(maxOf(1f, height).dp))
}

@Composable
private fun BriefPage(model: AppModel, taskId: String, stepId: String?, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    val client by model.client.collectAsState()
    val brief = remember(client) { client?.brief(taskId, stepId) }
    var messages by remember { mutableStateOf<List<BriefMessage>?>(null) }
    LaunchedEffect(client) {
        val c = client ?: return@LaunchedEffect
        messages = runCatching { c.briefMessages(taskId, stepId) }.getOrDefault(emptyList())
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar(brief?.title ?: "Brief", onBack) {
            brief?.threadChatId?.let { chat -> TextButton(onClick = { onOpenChat(chat) }) { Text("Open thread") } }
        }
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(top = 8.dp, bottom = 24.dp)) {
            if (brief == null) {
                Text("This brief is gone.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                return@Column
            }
            Text(brief.title, style = MaterialTheme.typography.titleLarge)
            if (brief.meta.isNotEmpty()) Text(brief.meta.joinToString(" · "), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(20.dp))
            @Composable fun section(name: String) = Text(name, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp, bottom = 6.dp))
            if (brief.task.isNotEmpty()) {
                section("TASK")
                Text(brief.task, style = MaterialTheme.typography.bodyLarge)
            }
            if (brief.spec.isNotEmpty()) {
                section("SPEC")
                PlanMarkdown(brief.spec)
            }
            section("MESSAGES")
            val got = messages
            when {
                got == null -> Text("Loading…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                got.isEmpty() -> Text(if (brief.threadChatId == null) "Not started yet." else "No messages.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                else -> for (m in got) {
                    Text(m.tag, style = MaterialTheme.typography.labelMedium, color = if (m.fromYou) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 12.dp))
                    Text(m.text, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(top = 3.dp))
                }
            }
        }
    }
}

@Composable
fun TreeOutline(model: AppModel, rootChatId: String, title: String, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar(title, onBack)
        TreeOutlineList(model, rootChatId, title, onOpenChat)
    }
}

@Composable
private fun TreeOutlineList(model: AppModel, rootChatId: String, title: String?, onOpenChat: (String) -> Unit) {
    val client by model.client.collectAsState()
    val ws by model.workspace.collectAsState()
    var tick by remember { mutableIntStateOf(0) }
    val nodes = remember(client, ws, tick) {
        val c = client ?: return@remember emptyList()
        runCatching { c.watchOutlineDocs(rootChatId) }
        c.taskOutline(rootChatId)
    }
    val live = nodes.any { it.status == TreeStatus.RUNNING || it.status == TreeStatus.WAITING }
    LaunchedEffect(live) {
        while (live) {
            delay(1000)
            tick++
        }
    }
    LaunchedEffect(client, rootChatId) {
        val c = client ?: return@LaunchedEffect
        while (true) {
            if (runCatching { c.refreshOutlineStats(rootChatId) }.getOrDefault(false)) tick++
            delay(10_000)
        }
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
        group("outline", null) {
            nodes.forEachIndexed { i, node ->
                if (i > 0) RowDivider()
                val indent = (node.depth.toInt() * 14).dp
                node.group?.let { g ->
                    Text(g, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 16.dp + indent, top = 10.dp, bottom = 4.dp))
                }
                val text = when (val k = node.kind) {
                    TreeNodeKind.Root -> title ?: node.title
                    is TreeNodeKind.Step -> "${k.n}. ${node.title}"
                    else -> node.title
                }
                val chat = node.chatId
                ListRow(
                    text,
                    listOfNotNull(node.word, node.time, node.tokens).joinToString(" · "),
                    glyph = glyph(node.status),
                    tint = if (node.muted) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                    strong = node.depth == 0u,
                    indent = indent,
                    chevron = chat != null,
                    onClick = chat?.let { { onOpenChat(it) } },
                    trailing = { BrandMarks(listOfNotNull(node.brand)) },
                )
            }
        }
    }
}
