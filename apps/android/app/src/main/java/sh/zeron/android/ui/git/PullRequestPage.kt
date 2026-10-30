package sh.zeron.android.ui.git

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.SectionHeader
import sh.zeron.android.ui.TonalCircleButton
import sh.zeron.android.ui.composerContainer
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.DiffFileView
import uniffi.zeron_core.PrActivity
import uniffi.zeron_core.PrControlsView
import uniffi.zeron_core.PrDetailView
import uniffi.zeron_core.PrHandoffKind
import uniffi.zeron_core.PrMergeMethod
import uniffi.zeron_core.PrReviewEvent
import uniffi.zeron_core.PrThread
import uniffi.zeron_core.PullRequestPage
import uniffi.zeron_core.pullRequestLink

private class Picker(val title: String, val search: String, val items: List<PickItem>, val toggle: (PickItem) -> Unit)
private class PickItem(val id: String, val title: String, val detail: String?, val on: Boolean)

@Composable
fun PullRequestPageScreen(model: AppModel, url: String, deviceId: String?, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    val client by model.client.collectAsState()
    val prefs = rememberGitModel()
    val device = remember(client, deviceId) { deviceId ?: client?.let { prefs.pickDevice(it.pullRequestDevices()) } }
    val page = remember(client, device) { if (client != null && device != null) client!!.pullRequestPage(device, url) else null }
    val number = remember(url) { pullRequestLink(url)?.number }
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var detail by remember(page) { mutableStateOf<PrDetailView?>(null) }
    var controls by remember(page) { mutableStateOf<PrControlsView?>(null) }
    var code by remember(page) { mutableStateOf<List<DiffFileView>?>(null) }
    var codeError by remember(page) { mutableStateOf<String?>(null) }
    var failure by remember(page) { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var input by rememberSaveable { mutableStateOf("") }
    var reload by remember { mutableIntStateOf(0) }
    var confirmMerge by remember { mutableStateOf<PrMergeMethod?>(null) }
    var replyTo by remember { mutableStateOf<PrThread?>(null) }
    var picker by remember { mutableStateOf<Picker?>(null) }
    val scope = rememberCoroutineScope()
    val uri = LocalUriHandler.current

    suspend fun loadCode(refresh: Boolean) {
        val p = page ?: return
        try {
            code = p.code(refresh).files
            codeError = null
        } catch (e: Exception) {
            codeError = GitText.failure(e, "Couldn't load the diff. Try again.")
        }
    }

    LaunchedEffect(page, reload) {
        val p = page ?: return@LaunchedEffect
        val refresh = reload > 0
        try {
            detail = p.load(refresh)
            failure = null
            if (controls == null || refresh) controls = runCatching { p.controls() }.getOrNull()
            if (refresh || tab == 1) loadCode(refresh)
        } catch (e: Exception) {
            failure = GitText.failure(e, "Couldn't load this pull request. Try again.")
        }
    }
    LaunchedEffect(tab, detail != null) { if (tab == 1 && code == null && codeError == null && detail != null) loadCode(false) }

    fun write(toast: String?, work: suspend (PullRequestPage) -> Unit) {
        val p = page ?: return
        if (busy) return
        busy = true
        scope.launch {
            try {
                work(p)
                toast?.let { model.toast(it) }
            } catch (e: Exception) {
                model.toast(GitText.failure(e, "GitHub refused the change. Try again."))
            }
            busy = false
        }
    }

    fun startSession(prompt: String) {
        val p = page ?: return
        scope.launch {
            val project = p.handoffProject()
            model.lastDraft = model.lastDraft.copy(projectId = project, hostId = if (project == null) device else null, cloud = false)
            model.newSessionText = prompt
            onOpenRoute("new")
        }
    }

    val d = detail
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            TonalCircleButton(ZIcons.Back, "Back", onClick = onBack)
            Text(
                (d?.number ?: number)?.let { "#$it" } ?: "Pull request",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f).padding(start = 16.dp),
            )
            TonalCircleButton(ZIcons.Refresh, "Refresh", onClick = { reload++ })
        }
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(bottom = 24.dp)) {
            item("head") {
                Column(Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (d != null) {
                        Text(d.title, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
                        Text("${d.repository} · ${d.author} · #${d.number} · Loaded just now", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        FilledTonalButton(onClick = { runCatching { page?.askContext() }.getOrNull()?.let { startSession(it.block + "\n\n") } }) {
                            ZIcon(ZIcons.Chat, null, Modifier.size(18.dp))
                            Text("Ask about this PR", Modifier.padding(start = 8.dp))
                        }
                    }
                    Segments(
                        listOf("Summary", d?.let { "Code ${it.fileCount}" } ?: "Code", d?.let { "Activity ${it.activity.size}" } ?: "Activity"),
                        tab,
                        { tab = it },
                        Modifier.padding(top = 4.dp),
                    )
                }
            }
            when {
                d == null && failure != null -> item("failure") { NoticeLine(failure!!, rememberDiffPalette().color(ColorRole.DANGER)) }
                d == null -> item("loading") { NoticeLine("Loading…", MaterialTheme.colorScheme.onSurfaceVariant) }
                tab == 0 -> summary(
                    d, controls,
                    onLabels = {
                        val c = controls ?: return@summary
                        picker = Picker("Change labels", "Search labels", c.labels.map { PickItem(it.name, it.name, null, it.applied) }) { item ->
                            write(if (item.on) "Removed ${item.title}" else "Added ${item.title}") { p -> controls = p.setLabel(item.id, !item.on) }
                        }
                    },
                    onReviewers = {
                        val c = controls ?: return@summary
                        picker = Picker("Request a review", "Search people with access", c.reviewers.map { PickItem(it.login, it.login, if (it.team) "Team" else null, it.requested) }) { item ->
                            val team = c.reviewers.firstOrNull { it.login == item.id }?.team ?: false
                            write(if (item.on) "Review request removed from ${item.title}" else "Review requested from ${item.title}") { p -> controls = p.setReviewer(item.id, team, !item.on) }
                        }
                    },
                    onMerge = { confirmMerge = it },
                    onHandoff = { kind -> runCatching { page?.handoffPrompt(kind) }.getOrNull()?.let(::startSession) },
                    onCheck = { u -> runCatching { uri.openUri(u) } },
                )
                tab == 1 -> {
                    val files = code
                    when {
                        codeError != null -> item("code-error") { DiffNotice(codeError!!) }
                        files == null -> item("code-loading") { DiffNotice("Loading the diff…") }
                        files.isEmpty() -> item("code-empty") { DiffNotice("No changes to show.") }
                        else -> diffItems(files, thread = { t -> ThreadCard(t, inline = true, onReply = { replyTo = t }, onResolve = { resolve(t, ::write) { detail = it } }) })
                    }
                }
                else -> {
                    if (d.activity.isEmpty()) {
                        item("no-activity") { NoticeLine("No comments or reviews yet.", MaterialTheme.colorScheme.onSurfaceVariant) }
                    } else {
                        if (d.openThreads > 0u) item("open") { SectionHeader("${d.openThreads} open conversation${if (d.openThreads == 1u) "" else "s"}") }
                        items(d.activity, key = { "activity:${it.id}" }) { a ->
                            ActivityCard(a, onReply = { t -> replyTo = t }, onResolve = { t -> resolve(t, ::write) { detail = it } })
                        }
                    }
                }
            }
        }
        if (tab == 2 && d != null) {
            CommentBar(
                text = input,
                onText = { input = it },
                busy = busy,
                canJudge = d.canJudge,
                onSend = {
                    val body = input.trim()
                    write("Comment posted") { p -> detail = p.comment(body); input = "" }
                },
                onReview = { event ->
                    val body = input.trim()
                    val toast = when (event) {
                        PrReviewEvent.APPROVE -> "Approved"
                        PrReviewEvent.REQUEST_CHANGES -> "Changes requested"
                        PrReviewEvent.COMMENT -> "Review submitted"
                    }
                    write(toast) { p -> detail = p.review(event, body); input = "" }
                },
            )
        }
    }

    confirmMerge?.let { method ->
        val confirm = remember(method) { page?.mergeConfirm(method) }
        if (confirm != null) {
            AlertDialog(
                onDismissRequest = { confirmMerge = null },
                title = { Text(confirm.title) },
                text = { Text(confirm.message) },
                confirmButton = {
                    TextButton(onClick = {
                        confirmMerge = null
                        write("Pull request merged") { p -> detail = p.merge(method) }
                    }) { Text(confirm.action) }
                },
                dismissButton = { TextButton(onClick = { confirmMerge = null }) { Text("Cancel") } },
            )
        }
    }
    replyTo?.let { thread ->
        var text by remember(thread) { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { replyTo = null },
            title = { Text("Reply") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(thread.line?.let { "${thread.path}:$it" } ?: thread.path, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    OutlinedTextField(text, { text = it }, placeholder = { Text("Reply…") }, modifier = Modifier.fillMaxWidth())
                }
            },
            confirmButton = {
                TextButton(enabled = text.isNotBlank(), onClick = {
                    val body = text.trim()
                    replyTo = null
                    write("Reply posted") { p -> detail = p.reply(thread.id, body) }
                }) { Text("Reply") }
            },
            dismissButton = { TextButton(onClick = { replyTo = null }) { Text("Cancel") } },
        )
    }
    picker?.let { p -> PickerSheet(p) { picker = null } }
}

private fun resolve(thread: PrThread, write: (String?, suspend (PullRequestPage) -> Unit) -> Unit, show: (PrDetailView) -> Unit) {
    write(if (thread.resolved) "Conversation unresolved" else "Conversation resolved") { p -> show(p.resolve(thread.id, !thread.resolved)) }
}

@Composable
private fun NoticeLine(text: String, color: Color) {
    Group(listOf(GroupRow("n") { Text(text, color = color, style = MaterialTheme.typography.bodyLarge) }), Modifier.padding(top = 8.dp))
}

private fun LazyListScope.summary(
    d: PrDetailView,
    controls: PrControlsView?,
    onLabels: () -> Unit,
    onReviewers: () -> Unit,
    onMerge: (PrMergeMethod) -> Unit,
    onHandoff: (PrHandoffKind) -> Unit,
    onCheck: (String) -> Unit,
) {
    val triage = controls?.canTriage == true
    item("overview") {
        SectionHeader("Overview")
        val labels = controls?.labels?.filter { it.applied }?.map { it.name }.orEmpty()
        val reviewers = controls?.reviewers?.filter { it.requested }?.map { it.login }.orEmpty()
        val muted = MaterialTheme.colorScheme.onSurfaceVariant
        Group(
            listOfNotNull(
                GroupRow("status") { InfoRow("Status") { PrChipLabel(d.state) } },
                d.review?.let { r -> GroupRow("review") { InfoRow("Review") { PrChipLabel(r) } } },
                GroupRow("branch") { InfoRow("Branch") { Text(d.branch, color = muted, maxLines = 1) } },
                GroupRow("changes") {
                    InfoRow("Changes") {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("${d.fileCount} file${if (d.fileCount == 1u) "" else "s"} · ", color = muted)
                            Text(changesText(d.changes))
                        }
                    }
                },
                GroupRow("labels", onClick = if (triage) onLabels else null) { InfoRow("Labels", triage) { Text(labels.ifEmpty { listOf("None") }.joinToString(", "), color = muted) } },
                GroupRow("reviewers", onClick = if (triage) onReviewers else null) { InfoRow("Reviewers", triage) { Text(reviewers.ifEmpty { listOf("None") }.joinToString(", "), color = muted) } },
            ),
        )
    }
    item("assessment") {
        SectionHeader("Current assessment")
        val palette = rememberDiffPalette()
        val muted = MaterialTheme.colorScheme.onSurfaceVariant
        Group(
            listOf(GroupRow("ci") { InfoRow("CI") { PrChipLabel(d.ci) } }) +
                d.blockers.mapIndexed { i, t -> GroupRow("b$i") { Text(t, color = palette.color(ColorRole.DANGER)) } } +
                d.attention.mapIndexed { i, t -> GroupRow("a$i") { Text(t, color = palette.color(ColorRole.WARNING)) } } +
                d.missing.mapIndexed { i, t -> GroupRow("m$i") { Text(t, color = muted) } } +
                listOfNotNull(d.handoffs.firstOrNull()?.let { h -> GroupRow("next") { Text("Next: ${h.label}", color = muted) } }),
        )
    }
    val methods = controls?.mergeMethods.orEmpty()
    if (d.isOpen && controls?.canWrite == true && methods.isNotEmpty()) {
        item("merge") {
            SectionHeader("Merge pull request")
            Group(methods.map { m -> GroupRow("merge:${m.method}", onClick = { onMerge(m.method) }) { ActionLabel(m.label, ZIcons.PullRequest) } })
            Footer("Into the base branch once you confirm.")
        }
    }
    item("checks") {
        SectionHeader("Checks · ${d.checks.size}")
        if (d.checks.isNotEmpty()) {
            Group(d.checks.mapIndexed { i, c ->
                GroupRow("check$i", onClick = c.url?.let { u -> { onCheck(u) } }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(c.name, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                        PrChipLabel(c.chip)
                    }
                }
            })
        }
        d.checksEmpty?.let { Footer(it) }
    }
    if (d.handoffs.isNotEmpty()) {
        item("handoff") {
            SectionHeader("Hand off to an agent")
            Group(d.handoffs.map { h -> GroupRow("handoff:${h.kind}", onClick = { onHandoff(h.kind) }) { ActionLabel(h.label, ZIcons.Bot) } })
            Footer("Opens a new session with the prompt ready to send.")
        }
    }
    if (d.body.isNotBlank()) {
        item("body") {
            SectionHeader("Description")
            Group(listOf(GroupRow("body") { Text(d.body, style = MaterialTheme.typography.bodyLarge) }))
        }
    }
}

@Composable
private fun ActionLabel(label: String, icon: Int) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        ZIcon(icon, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
        Text(label, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(start = 14.dp))
    }
}

@Composable
private fun Footer(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 28.dp, vertical = 8.dp))
}

@Composable
private fun ActivityCard(item: PrActivity, onReply: (PrThread) -> Unit, onResolve: (PrThread) -> Unit) {
    Box(Modifier.padding(horizontal = 12.dp).padding(bottom = 8.dp)) {
        Surface(shape = RoundedCornerShape(20.dp), color = sh.zeron.android.ui.cardColor(), modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(item.comment.author, style = MaterialTheme.typography.titleSmall)
                    item.state?.let { PrChipLabel(it) }
                    Text(item.comment.`when`, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
                }
                val thread = item.thread
                if (thread != null) {
                    val place = thread.line?.let { "${thread.path}:$it" } ?: thread.path
                    val state = if (thread.resolved) "Resolved" else if (thread.outdated) "Outdated" else "Unresolved"
                    Text("$place · $state", style = MaterialTheme.typography.labelMedium, fontFamily = sh.zeron.android.design.GeistMono, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (item.comment.body.isNotBlank()) Text(item.comment.body, style = MaterialTheme.typography.bodyLarge)
                if (thread != null) {
                    for (reply in thread.comments.drop(1)) {
                        Column(Modifier.padding(start = 12.dp, top = 4.dp)) {
                            Text("${reply.author} · ${reply.`when`}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Text(reply.body, style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                    ThreadButtons(thread, onReply, onResolve)
                }
            }
        }
    }
}

@Composable
private fun ThreadButtons(thread: PrThread, onReply: (PrThread) -> Unit, onResolve: (PrThread) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        TextButton(onClick = { onReply(thread) }) { Text("Reply") }
        TextButton(onClick = { onResolve(thread) }) { Text(if (thread.resolved) "Unresolve" else "Resolve") }
    }
}

@Composable
private fun ThreadCard(thread: PrThread, inline: Boolean, onReply: () -> Unit, onResolve: () -> Unit) {
    val palette = rememberDiffPalette()
    Column(Modifier.fillMaxWidth().background(palette.color(ColorRole.CARD_BACKGROUND)).padding(start = if (inline) 66.dp else 16.dp, end = 12.dp, top = 8.dp, bottom = 4.dp)) {
        Text("${thread.comments.firstOrNull()?.author.orEmpty()} · ${if (thread.resolved) "Resolved" else "Unresolved"}", style = MaterialTheme.typography.labelLarge)
        val ink = MaterialTheme.colorScheme.onSurface
        for (c in thread.comments) {
            Text(
                buildAnnotatedString {
                    withStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = ink)) { append(c.author) }
                    append(": ${c.body}")
                },
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
        ThreadButtons(thread, { onReply() }, { onResolve() })
    }
}

@Composable
private fun CommentBar(text: String, onText: (String) -> Unit, busy: Boolean, canJudge: Boolean, onSend: () -> Unit, onReview: (PrReviewEvent) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val empty = text.isBlank()
    Surface(color = composerContainer()) {
        Row(Modifier.fillMaxWidth().imePadding().navigationBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextField(
                text,
                onText,
                placeholder = { Text("Write a comment…") },
                maxLines = 5,
                shape = RoundedCornerShape(20.dp),
                colors = TextFieldDefaults.colors(focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent),
                modifier = Modifier.weight(1f).heightIn(min = 48.dp),
            )
            Box {
                FilledTonalButton(onClick = { menu = true }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Review") }
                ChoiceMenu(
                    menu,
                    { menu = false },
                    listOf(
                        MenuChoice("Approve", enabled = canJudge && !busy) { onReview(PrReviewEvent.APPROVE) },
                        MenuChoice("Request changes", enabled = canJudge && !empty && !busy) { onReview(PrReviewEvent.REQUEST_CHANGES) },
                        MenuChoice("Comment as review", enabled = !empty && !busy) { onReview(PrReviewEvent.COMMENT) },
                    ),
                    title = "The text in the box becomes the review summary.",
                )
            }
            FilledIconButton(onClick = onSend, enabled = !empty && !busy, modifier = Modifier.size(48.dp)) { ZIcon(ZIcons.Send, "Send", Modifier.size(20.dp)) }
        }
    }
}

@Composable
private fun PickerSheet(picker: Picker, onDismiss: () -> Unit) {
    var query by remember { mutableStateOf("") }
    val on = remember(picker) { mutableStateListOf<String>().apply { addAll(picker.items.filter { it.on }.map { it.id }) } }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(picker.title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                TextButton(onClick = onDismiss) { Text("Done") }
            }
            TextField(
                query,
                { query = it },
                placeholder = { Text(picker.search) },
                leadingIcon = { ZIcon(ZIcons.Search, null, Modifier.size(20.dp)) },
                singleLine = true,
                shape = RoundedCornerShape(50),
                colors = TextFieldDefaults.colors(focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            )
            val shown = picker.items.filter { query.isEmpty() || it.title.contains(query, ignoreCase = true) }
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 480.dp), contentPadding = PaddingValues(bottom = 16.dp)) {
                item("list") {
                    Group(shown.map { item ->
                        val checked = item.id in on
                        GroupRow(item.id, onClick = {
                            picker.toggle(PickItem(item.id, item.title, item.detail, checked))
                            if (checked) on.remove(item.id) else on.add(item.id)
                        }) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f)) {
                                    Text(item.title, style = MaterialTheme.typography.bodyLarge)
                                    item.detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                                }
                                if (checked) ZIcon(ZIcons.Check, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                            }
                        }
                    })
                }
            }
        }
    }
}
