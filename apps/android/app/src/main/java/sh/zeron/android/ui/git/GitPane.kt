package sh.zeron.android.ui.git

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ProjectColors
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.SectionHeader
import sh.zeron.android.ui.TonalCircleButton
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentShape
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.GitCheckout
import uniffi.zeron_core.GitCommitView
import uniffi.zeron_core.GitFileRow
import uniffi.zeron_core.GitHistoryRow
import uniffi.zeron_core.GitHistoryView
import uniffi.zeron_core.GitSegmentShape
import uniffi.zeron_core.GitTreeView

private class OpenDiff(val file: GitFileRow)

@Composable
fun GitPane(
    model: AppModel,
    client: CoreClient,
    deviceId: String,
    projectId: String?,
    startHistory: Boolean,
    refresh: Int,
    onOpenCommit: (GitCheckout, GitHistoryRow) -> Unit,
) {
    val workspace by model.workspace.collectAsState()
    var checkoutPath by remember(projectId) { mutableStateOf<String?>(null) }
    val checkout = remember(projectId, checkoutPath) { projectId?.let { runCatching { client.gitCheckout(it, checkoutPath) }.getOrNull() } }
    var tree by remember(checkout) { mutableStateOf<GitTreeView?>(null) }
    var history by remember(checkout) { mutableStateOf<GitHistoryView?>(null) }
    var failure by remember(checkout) { mutableStateOf<String?>(null) }
    var showingHistory by remember { mutableStateOf(startHistory) }
    var busy by remember { mutableStateOf(false) }
    var confirmPush by remember { mutableStateOf(false) }
    var committing by remember { mutableStateOf(false) }
    var diff by remember { mutableStateOf<OpenDiff?>(null) }
    val scope = rememberCoroutineScope()

    fun show(next: GitTreeView) {
        val notice = next.notice
        tree = if (notice != null && checkout != null) {
            model.toast(notice.text)
            checkout.dismissNotice()
        } else {
            next
        }
    }

    LaunchedEffect(checkout, refresh) {
        val c = checkout ?: return@LaunchedEffect
        try {
            show(c.refresh())
            failure = null
            if (showingHistory || history != null) history = c.history(false)
        } catch (e: Exception) {
            failure = GitText.failure(e, "Couldn't read this checkout. Try again.")
        }
    }
    LaunchedEffect(checkout, showingHistory) {
        val c = checkout ?: return@LaunchedEffect
        if (showingHistory && history == null) {
            try {
                history = c.history(false)
            } catch (e: Exception) {
                failure = GitText.failure(e, "Couldn't read the history. Try again.")
            }
        }
    }

    fun act(work: suspend (GitCheckout) -> GitTreeView) {
        val c = checkout ?: return
        if (busy) return
        busy = true
        scope.launch {
            try {
                show(work(c))
                if (history != null) history = runCatching { c.history(false) }.getOrNull() ?: history
            } catch (e: Exception) {
                model.toast(GitText.failure(e, "Git refused the change. Try again."))
            }
            busy = false
        }
    }

    val options = remember(projectId, workspace) { projectId?.let { client.gitCheckouts(it) }.orEmpty() }
    val deviceName = workspace?.devices?.firstOrNull { it.id == deviceId }?.name ?: "Mac"
    val projectName = workspace?.projects?.firstOrNull { it.id == projectId }?.name.orEmpty()
    val place = "$deviceName · $projectName · ${if (checkoutPath == null) "main checkout" else "agent worktree"}"
    val t = tree

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 12.dp, bottom = 32.dp)) {
        item("git-head") {
            Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(
                    t?.branch?.takeIf { it.isNotEmpty() } ?: options.firstOrNull { it.path == checkoutPath }?.label ?: "Git",
                    fontFamily = GeistMono,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 20.sp,
                )
                if (t != null) {
                    Text(
                        "${t.ahead} ahead · ${t.behind} behind" + (t.upstream?.let { " · $it" } ?: ""),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                CheckoutPicker(place, options, checkoutPath) { checkoutPath = it }
                t?.agentBanner?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = rememberDiffPalette().color(ColorRole.WARNING)) }
                Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(
                        onClick = { val r = t?.pullRefusal; if (r != null) model.toast(r) else act { it.pull() } },
                        enabled = t != null && !busy,
                    ) { ButtonLabel("Pull", ZIcons.ArrowDown) }
                    FilledTonalButton(
                        onClick = { val r = t?.pushRefusal; if (r != null) model.toast(r) else confirmPush = true },
                        enabled = t != null && !busy,
                    ) { ButtonLabel(if ((t?.ahead ?: 0u) > 0u) "Push ${t!!.ahead}" else "Push", ZIcons.ArrowUp) }
                    Button(onClick = { committing = true }, enabled = t != null && !busy) { ButtonLabel("Commit", ZIcons.Check) }
                }
                Segments(listOf("Working tree", "History"), if (showingHistory) 1 else 0, { showingHistory = it == 1 }, Modifier.padding(top = 6.dp))
            }
        }
        when {
            projectId == null -> item("no-project") { Box(Modifier.padding(top = 12.dp)) { NoticeCard("No Git project on this Mac. Add a project with a Git checkout first.", "") } }
            t == null -> item("reading") { Box(Modifier.padding(top = 12.dp)) { NoticeCard(failure ?: "Reading the checkout…", "") } }
            showingHistory -> {
                val h = history
                if (h == null) {
                    item("reading-history") { Box(Modifier.padding(top = 12.dp)) { NoticeCard(failure ?: "Reading the history…", "") } }
                } else {
                    item("history-head") { SectionHeader("${h.rows.size} commits · ${t.ahead} ahead · ${t.behind} behind") }
                    val pending = t.staged.size + t.changes.size
                    val count = h.rows.size + (if (pending > 0) 1 else 0) + (if (h.canLoadMore) 1 else 0)
                    var index = 0
                    if (pending > 0) {
                        val i = index++
                        item("uncommitted") {
                            Cell(i, count) { Text("Uncommitted changes ($pending)", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        }
                    }
                    val base = index
                    itemsIndexed(h.rows, key = { _, r -> "commit:${r.sha}" }) { i, row ->
                        Cell(base + i, count, padded = false, onClick = { checkout?.let { onOpenCommit(it, row) } }) { HistoryRow(row, h.laneCount.toInt()) }
                    }
                    if (h.canLoadMore) {
                        item("history-more") {
                            Cell(count - 1, count, onClick = {
                                scope.launch { history = runCatching { checkout?.history(true) }.getOrNull() ?: history }
                            }) { Text("Load more", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall) }
                        }
                    }
                }
            }
            else -> {
                val clean = t.clean
                if (clean != null) {
                    item("clean") { Box(Modifier.padding(top = 12.dp)) { NoticeCard(clean, "") } }
                } else {
                    fileSection("staged", "Staged · ${t.staged.size}", t.staged, "No staged files", if (t.staged.isEmpty()) null else "Unstage all", busy, { act { it.unstageAll() } }, { diff = OpenDiff(it) }) { f -> act { it.unstage(listOf(f.path)) } }
                    fileSection("changes", "Changes · ${t.changes.size}", t.changes, "No changes", if (t.changes.isEmpty()) null else "Stage all", busy, { act { it.stageAll() } }, { diff = OpenDiff(it) }) { f -> act { it.stage(listOf(f.path)) } }
                }
                if (t.stashes.isNotEmpty()) {
                    item("stashes-head") { SectionHeader("Stashes · ${t.stashes.size}") }
                    itemsIndexed(t.stashes, key = { i, _ -> "stash:$i" }) { i, s ->
                        Cell(i, t.stashes.size) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                ZIcon(ZIcons.Archive, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                Text(s, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(start = 12.dp))
                            }
                        }
                    }
                }
            }
        }
        item("git-pad") { Box(Modifier.navigationBarsPadding()) }
    }

    if (confirmPush && t != null) {
        AlertDialog(
            onDismissRequest = { confirmPush = false },
            title = { Text(t.pushPrompt ?: "Push ${t.branch}?") },
            text = { Text(place) },
            confirmButton = {
                TextButton(onClick = {
                    confirmPush = false
                    act { it.push() }
                }) { Text("Push") }
            },
            dismissButton = { TextButton(onClick = { confirmPush = false }) { Text("Cancel") } },
        )
    }
    if (committing && t != null && checkout != null) {
        CommitSheet(model, checkout, t, place, onDismiss = { committing = false }) { next ->
            committing = false
            show(next)
            if (history != null) scope.launch { history = runCatching { checkout.history(false) }.getOrNull() ?: history }
        }
    }
    diff?.let { open ->
        val f = open.file
        val dir = f.path.substringBeforeLast('/', "")
        DiffSheet(f.path.substringAfterLast('/'), "$dir · ${if (f.staged) "Staged" else "Unstaged"}", onDismiss = { diff = null }) {
            checkout?.fileDiff(f.path, f.staged).orEmpty()
        }
    }
}

@Composable
private fun ButtonLabel(text: String, icon: Int) {
    ZIcon(icon, null, Modifier.size(16.dp))
    Text(text, Modifier.padding(start = 6.dp))
}

@Composable
private fun Cell(index: Int, count: Int, padded: Boolean = true, onClick: (() -> Unit)? = null, content: @Composable () -> Unit) {
    Box(Modifier.padding(horizontal = 12.dp).padding(bottom = 2.dp)) {
        val shape = segmentShape(index, count)
        if (padded) {
            GroupCell(shape, onClick, content)
        } else if (onClick != null) {
            Surface(onClick = onClick, shape = shape, color = cardColor(), modifier = Modifier.fillMaxWidth()) { content() }
        } else {
            Surface(shape = shape, color = cardColor(), modifier = Modifier.fillMaxWidth()) { content() }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.fileSection(
    key: String,
    title: String,
    files: List<GitFileRow>,
    none: String,
    action: String?,
    busy: Boolean,
    onAction: () -> Unit,
    onOpen: (GitFileRow) -> Unit,
    onToggle: (GitFileRow) -> Unit,
) {
    item("$key-head") {
        SectionHeader(title, trailing = action?.let { label -> { TextButton(onClick = onAction, enabled = !busy) { Text(label) } } })
    }
    if (files.isEmpty()) {
        item("$key-none") { Cell(0, 1) { Text(none, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.outline) } }
    }
    itemsIndexed(files, key = { _, f -> "$key:${f.path}" }) { i, f ->
        Cell(i, files.size, onClick = { onOpen(f) }) { FileRow(f, busy) { onToggle(f) } }
    }
}

@Composable
private fun letterColor(letter: String): Color {
    val p = rememberDiffPalette()
    return when (letter) {
        "A", "?", "U" -> p.color(ColorRole.SUCCESS)
        "D" -> p.color(ColorRole.DANGER)
        "R", "C" -> p.color(ColorRole.ACCENT)
        else -> p.color(ColorRole.WARNING)
    }
}

@Composable
private fun FileRow(file: GitFileRow, busy: Boolean, onToggle: () -> Unit) {
    val name = file.path.substringAfterLast('/')
    val dir = file.path.substringBeforeLast('/', "")
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(file.letter, fontFamily = GeistMono, fontWeight = FontWeight.SemiBold, fontSize = 13.sp, color = letterColor(file.letter), modifier = Modifier.width(22.dp))
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (dir.isNotEmpty()) Text("$dir  ", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                DiffDelta(file.additions, file.deletions)
            }
        }
        FilledTonalButton(onClick = onToggle, enabled = !busy, contentPadding = PaddingValues(horizontal = 14.dp), modifier = Modifier.height(34.dp)) {
            Text(if (file.staged) "Unstage" else "Stage", style = MaterialTheme.typography.labelLarge)
        }
    }
}

@Composable
private fun CheckoutPicker(place: String, options: List<uniffi.zeron_core.GitCheckoutOption>, current: String?, onPick: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }, contentPadding = PaddingValues(0.dp)) {
            Text(place, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            ZIcon(ZIcons.ChevronDown, null, Modifier.padding(start = 4.dp).size(14.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        val main = options.filter { it.path == null }
        val worktrees = options.filter { it.path != null }
        androidx.compose.material3.DropdownMenu(expanded = open, onDismissRequest = { open = false }, shape = RoundedCornerShape(16.dp)) {
            MenuTitle("Main checkout")
            for (o in main) PickerItem(o.label, o.detail, o.path == current, false) { open = false; onPick(null) }
            MenuTitle("Agent worktrees")
            if (worktrees.isEmpty()) {
                androidx.compose.material3.DropdownMenuItem(text = { Text("No agent worktrees") }, enabled = false, onClick = {})
            }
            for (o in worktrees) PickerItem(o.label, o.detail, o.path == current, o.live) { open = false; onPick(o.path) }
        }
    }
}

@Composable
private fun MenuTitle(text: String) {
    Text(text, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
}

@Composable
private fun PickerItem(label: String, detail: String, checked: Boolean, live: Boolean, onClick: () -> Unit) {
    val success = rememberDiffPalette().color(ColorRole.SUCCESS)
    androidx.compose.material3.DropdownMenuItem(
        text = {
            Column {
                Text(label)
                if (detail.isNotEmpty()) Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        },
        leadingIcon = if (live) ({ Box(Modifier.size(8.dp).background(success, androidx.compose.foundation.shape.CircleShape)) }) else null,
        trailingIcon = if (checked) ({ ZIcon(ZIcons.Check, null, Modifier.size(18.dp)) }) else null,
        onClick = onClick,
    )
}

private const val LANE = 12f

@Composable
private fun HistoryRow(row: GitHistoryRow, lanes: Int) {
    val dark = LocalDarkTheme.current
    val background = MaterialTheme.colorScheme.background
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
        Canvas(Modifier.padding(start = 10.dp).width((maxOf(1, minOf(lanes, 6)) * LANE).dp).fillMaxHeight()) {
            val lane = LANE.dp.toPx()
            val mid = size.height / 2
            fun x(l: UInt) = l.toFloat() * lane + lane / 2
            fun color(id: UInt) = ProjectColors.color(id.toInt(), dark)
            val width = 1.5.dp.toPx()
            for (s in row.segments) {
                val effect = if (s.dashed) PathEffect.dashPathEffect(floatArrayOf(3.dp.toPx(), 3.dp.toPx())) else null
                val (from, to) = when (s.shape) {
                    GitSegmentShape.THROUGH -> Offset(x(s.fromLane), 0f) to Offset(x(s.toLane), size.height)
                    GitSegmentShape.INCOMING -> Offset(x(s.fromLane), 0f) to Offset(x(s.toLane), mid)
                    GitSegmentShape.OUTGOING -> Offset(x(s.fromLane), mid) to Offset(x(s.toLane), size.height)
                }
                drawLine(color(s.color), from, to, width, pathEffect = effect)
            }
            val r = (if (row.isHead) 4.5f else 3.5f).dp.toPx()
            val center = Offset(x(row.lane), mid)
            drawCircle(if (row.isHead) background else color(row.color), r, center)
            if (row.isHead) drawCircle(color(row.color), r, center, style = Stroke(2.dp.toPx()))
        }
        Column(Modifier.weight(1f).padding(start = 8.dp, end = 12.dp, top = 10.dp, bottom = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(row.subject, style = MaterialTheme.typography.titleSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(historyMeta(row), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun historyMeta(row: GitHistoryRow): AnnotatedString {
    val p = rememberDiffPalette()
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    return buildAnnotatedString {
        for (chip in row.refs) {
            val c = when (chip.kind) {
                "remote" -> muted
                "tag" -> p.color(ColorRole.WARNING)
                else -> p.color(ColorRole.ACCENT)
            }
            withStyle(SpanStyle(color = c, background = c.copy(alpha = 0.12f), fontFamily = GeistMono, fontWeight = FontWeight.Medium, fontSize = 11.sp)) { append(chip.label) }
            append(" ")
        }
        append("${row.author} · ${row.`when`} · ${row.shortSha}")
    }
}

@Composable
fun CommitDetail(model: AppModel, checkout: GitCheckout, sha: String, short: String, onBack: () -> Unit) {
    var commit by remember(sha) { mutableStateOf<GitCommitView?>(null) }
    var failure by remember(sha) { mutableStateOf<String?>(null) }
    val clipboard = LocalClipboardManager.current
    LaunchedEffect(sha) {
        try {
            commit = checkout.commitDetail(sha)
        } catch (e: Exception) {
            failure = GitText.failure(e, "Couldn't load this commit. Try again.")
        }
    }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            TonalCircleButton(ZIcons.Back, "Back", onClick = onBack)
            Text("Commit $short", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f).padding(start = 16.dp))
            FilledTonalButton(onClick = {
                clipboard.setText(AnnotatedString(sha))
                model.toast("Copied ${sha.take(8)}")
            }) {
                ZIcon(ZIcons.Copy, null, Modifier.size(16.dp))
                Text("Copy SHA", Modifier.padding(start = 6.dp))
            }
        }
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(top = 12.dp, bottom = 24.dp)) {
            val c = commit
            if (c == null) {
                item("commit-loading") { DiffNotice(failure ?: "Loading…") }
            } else {
                item("commit-info") {
                    Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(c.subject, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                        for ((label, value) in listOf("Author" to c.author, "Date" to c.`when`, "SHA" to c.sha, "Files" to "${c.files.size}")) {
                            Row {
                                Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.width(72.dp))
                                Text(value, style = MaterialTheme.typography.bodyMedium, fontFamily = if (label == "SHA") GeistMono else null, fontSize = if (label == "SHA") 12.sp else MaterialTheme.typography.bodyMedium.fontSize)
                            }
                        }
                    }
                }
                diffItems(c.files)
                item("commit-pad") { Box(Modifier.navigationBarsPadding()) }
            }
        }
    }
}

private enum class CommitKind { Commit, Amend, All }

@Composable
private fun CommitSheet(model: AppModel, checkout: GitCheckout, start: GitTreeView, place: String, onDismiss: () -> Unit, onDone: (GitTreeView) -> Unit) {
    var tree by remember { mutableStateOf(start) }
    var message by remember { mutableStateOf("") }
    var kind by remember { mutableStateOf(CommitKind.Commit) }
    var busy by remember { mutableStateOf(false) }
    var writing by remember { mutableStateOf(false) }
    var written by remember { mutableStateOf(false) }
    var more by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val subject = message.substringBefore('\n')
    val files = tree.staged.size
    val empty = message.isBlank()
    val blocked = when (kind) {
        CommitKind.Commit -> tree.commitRefusal != null
        CommitKind.Amend -> !tree.canAmend
        CommitKind.All -> tree.staged.isEmpty() && tree.changes.isEmpty()
    }
    val title = when (kind) {
        CommitKind.Commit -> "Commit $files file${if (files == 1) "" else "s"}"
        CommitKind.Amend -> "Amend"
        CommitKind.All -> "Commit all"
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().imePadding().navigationBarsPadding().padding(horizontal = 20.dp).padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Commit", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                TextButton(onClick = onDismiss) { Text("Cancel") }
            }
            Text("${tree.branch} · $place", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Commit message", style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                Text(
                    "${subject.length}/${tree.subjectLimit}",
                    fontFamily = GeistMono,
                    fontSize = 12.sp,
                    color = if (subject.length > tree.subjectLimit.toInt()) rememberDiffPalette().color(ColorRole.WARNING) else MaterialTheme.colorScheme.outline,
                )
            }
            TextField(
                message,
                { message = it },
                shape = RoundedCornerShape(16.dp),
                colors = TextFieldDefaults.colors(focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent),
                modifier = Modifier.fillMaxWidth().height(140.dp),
            )
            FilledTonalButton(
                onClick = {
                    busy = true
                    writing = true
                    scope.launch {
                        try {
                            message = checkout.generateMessage()
                            written = true
                        } catch (e: Exception) {
                            model.toast(GitText.failure(e, "Couldn't write a message. Try again."))
                        }
                        busy = false
                        writing = false
                    }
                },
                enabled = !busy && tree.staged.isNotEmpty(),
            ) {
                ZIcon(ZIcons.Magic, null, Modifier.size(18.dp))
                Text(if (writing) "Writing…" else if (written) "Rewrite" else "Write with AI", Modifier.padding(start = 8.dp))
            }
            if (written) {
                Text("Written from the staged diff by the Mac's commit model. Edit it before you commit.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Button(
                    onClick = {
                        val text = message.trim()
                        busy = true
                        val k = kind
                        scope.launch {
                            try {
                                val next = checkout.commit(text, k == CommitKind.Amend, k == CommitKind.All)
                                busy = false
                                if (next.notice?.error == true) {
                                    tree = checkout.dismissNotice()
                                    model.toast(next.notice?.text.orEmpty())
                                } else {
                                    onDone(next)
                                }
                            } catch (e: Exception) {
                                busy = false
                                model.toast(GitText.failure(e, "Couldn't commit. Try again."))
                            }
                        }
                    },
                    enabled = !busy && !empty && !blocked,
                    modifier = Modifier.weight(1f),
                ) { Text(if (busy && !writing) "Committing…" else title) }
                Box {
                    FilledTonalIconButton(onClick = { more = true }) { ZIcon(ZIcons.ChevronDown, "More commit options", Modifier.size(20.dp)) }
                    ChoiceMenu(
                        more,
                        { more = false },
                        listOf(
                            MenuChoice("Commit", "Commit the staged files", kind == CommitKind.Commit) { kind = CommitKind.Commit },
                            MenuChoice("Amend", "Fold into the previous commit (not pushed)", kind == CommitKind.Amend, enabled = tree.canAmend) { kind = CommitKind.Amend },
                            MenuChoice("Commit all", "Stage everything, then commit", kind == CommitKind.All) { kind = CommitKind.All },
                        ),
                        title = "More commit options",
                    )
                }
            }
        }
    }
}
