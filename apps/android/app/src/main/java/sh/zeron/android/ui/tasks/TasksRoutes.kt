package sh.zeron.android.ui.tasks

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.TasksText
import sh.zeron.android.core.rememberTasks
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.TonalCircleButton
import uniffi.zeron_core.KanbanColumn
import uniffi.zeron_core.TaskRow
import uniffi.zeron_core.TaskStatus
import uniffi.zeron_core.TreeIndexRow
import uniffi.zeron_core.TreeNodeKind
import uniffi.zeron_core.TreeStatus
import uniffi.zeron_core.taskDropCompletes
import uniffi.zeron_core.tasksViewLabels
import uniffi.zeron_core.treeLabels

private const val PREFS = "tasks"
private const val VIEW_KEY = "tasksView"

@Composable
fun TasksScreen(
    model: AppModel,
    onOpenChat: (String) -> Unit,
    openTask: String? = null,
    onBack: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS, Context.MODE_PRIVATE) }
    var kanban by rememberSaveable { mutableStateOf(prefs.getString(VIEW_KEY, "list") == "kanban") }
    var sheetTask by rememberSaveable { mutableStateOf(openTask) }
    var newTask by rememberSaveable { mutableStateOf(false) }
    var tree by rememberSaveable { mutableStateOf<Pair<String, String>?>(null) }
    LaunchedEffect(openTask) { if (openTask != null) sheetTask = openTask }
    val snap = rememberTasks(model)
    tree?.let { (root, title) ->
        BackHandler { tree = null }
        TreeOutline(model, root, title, onBack = { tree = null }, onOpenChat = onOpenChat)
        return
    }
    val scope = rememberCoroutineScope()
    val page = snap.page
    val labels = remember { tasksViewLabels() }
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            if (onBack != null) {
                TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, size = 44.dp)
                Spacer(Modifier.width(12.dp))
            }
            SingleChoiceSegmentedButtonRow(Modifier.width(200.dp)) {
                labels.take(2).forEachIndexed { i, label ->
                    SegmentedButton(
                        selected = (i == 1) == kanban,
                        onClick = {
                            kanban = i == 1
                            prefs.edit().putString(VIEW_KEY, if (kanban) "kanban" else "list").apply()
                        },
                        shape = SegmentedButtonDefaults.itemShape(i, 2),
                        icon = {},
                    ) { Text(label) }
                }
            }
            Spacer(Modifier.weight(1f))
            TonalCircleButton(ZIcons.Plus, "New task", onClick = { newTask = true }, size = 44.dp)
        }
        Text(
            "Tasks",
            style = MaterialTheme.typography.displaySmallEmphasized,
            modifier = Modifier.padding(start = 24.dp, top = 8.dp, bottom = 8.dp),
        )
        val open: (TaskRow) -> Unit = { row -> row.leadChatId?.let(onOpenChat) ?: run { sheetTask = row.id } }
        val details: (TaskRow) -> Unit = { row -> sheetTask = row.id }
        if (kanban) {
            KanbanBoard(page.board, onOpen = open, onDetails = details, onMove = { row, completed ->
                val client = snap.client ?: return@KanbanBoard
                scope.launch {
                    runCatching { client.setTaskCompleted(row.id, completed) }.onFailure { model.toast(TasksText.error(it)) }
                }
            })
        } else {
            val treeBadge = remember { treeLabels().badge }
            val sections = remember(page, snap.trees) {
                listOf(
                    TaskStatus.NEEDS_YOU to page.needsYou,
                    TaskStatus.WORKING to page.working,
                    TaskStatus.TO_DO to page.toDo,
                    TaskStatus.DONE to page.done,
                ).mapNotNull { (status, rows) ->
                    val trees = when (status) {
                        TaskStatus.WORKING -> snap.trees.running
                        TaskStatus.DONE -> snap.trees.doneToday
                        else -> emptyList()
                    }
                    val count = rows.size + trees.size
                    if (count == 0) null else Triple(status, rows, trees)
                }
            }
            if (sections.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        "No tasks yet.\nTap + to add one.",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    )
                }
            } else {
                LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 4.dp, bottom = 32.dp)) {
                    for ((status, rows, trees) in sections) {
                        val title = page.board.firstOrNull { it.status == status }?.title.orEmpty()
                        group("status-$status", "$title · ${rows.size + trees.size}") {
                            rows.forEachIndexed { i, task ->
                                if (i > 0) RowDivider()
                                ListRow(task.title, TasksText.taskLine(task), glyph = glyph(task.status), chevron = true, onClick = { open(task) }, onLongClick = { details(task) })
                            }
                            trees.forEachIndexed { i, t ->
                                if (i > 0 || rows.isNotEmpty()) RowDivider()
                                TreeRow(t, treeBadge) { tree = t.root to t.title }
                            }
                        }
                    }
                }
            }
        }
    }
    sheetTask?.let { id ->
        TaskSheet(
            model,
            id,
            onDismiss = { sheetTask = null },
            onOpenThread = { chat ->
                sheetTask = null
                onOpenChat(chat)
            },
        )
    }
    if (newTask) {
        NewTaskSheet(model, onDismiss = { newTask = false }) { taskId ->
            newTask = false
            sheetTask = taskId
        }
    }
}

@Composable
fun RowDivider() {
    HorizontalDivider(Modifier.padding(start = 44.dp), color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
}

@Composable
private fun TreeRow(tree: TreeIndexRow, badge: String, onClick: () -> Unit) {
    ListRow(
        tree.title,
        listOfNotNull(badge, tree.line, tree.agents).joinToString(" · "),
        glyph = glyph(tree.status),
        chevron = true,
        onClick = onClick,
    )
}

@Composable
fun rememberTasksBadge(model: AppModel): Int = rememberTasks(model).page.needsYouCount.toInt()

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun KanbanBoard(board: List<KanbanColumn>, onOpen: (TaskRow) -> Unit, onDetails: (TaskRow) -> Unit, onMove: (TaskRow, Boolean) -> Unit) {
    val width = (LocalConfiguration.current.screenWidthDp * 0.78f).dp
    Row(
        Modifier.fillMaxSize().horizontalScroll(rememberScrollState()).padding(start = 16.dp, end = 4.dp, top = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        for (column in board) {
            Column(Modifier.width(width).fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(Modifier.padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(8.dp).clip(CircleShape).background(columnTone(column.status)))
                    Spacer(Modifier.width(8.dp))
                    Text(column.title, style = MaterialTheme.typography.titleSmall)
                    Spacer(Modifier.width(8.dp))
                    Text("${column.rows.size}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                for (row in column.rows) {
                    var menu by remember { mutableStateOf(false) }
                    Box {
                        KanbanCard(row, Modifier.combinedClickable(onClick = { onOpen(row) }, onLongClick = { menu = true }))
                        DropdownMenu(menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(text = { Text("Open") }, onClick = {
                                menu = false
                                onOpen(row)
                            })
                            if (row.leadChatId != null) {
                                DropdownMenuItem(text = { Text("Details") }, onClick = {
                                    menu = false
                                    onDetails(row)
                                })
                            }
                            for (target in board) {
                                val completes = taskDropCompletes(row.status, target.status) ?: continue
                                DropdownMenuItem(text = { Text("Move to ${target.title}") }, onClick = {
                                    menu = false
                                    onMove(row, completes)
                                })
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun columnTone(status: TaskStatus) = when (status) {
    TaskStatus.TO_DO -> Tones.idle
    TaskStatus.WORKING -> Tones.working
    TaskStatus.NEEDS_YOU -> Tones.input
    TaskStatus.DONE -> Tones.done
}

@Composable
private fun KanbanCard(row: TaskRow, modifier: Modifier) {
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth().border(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f), RoundedCornerShape(12.dp)).clip(RoundedCornerShape(12.dp)).then(modifier),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                row.title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
                color = if (row.status == TaskStatus.DONE) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )
            Text(
                TasksText.taskLine(row),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
fun TreeOutline(model: AppModel, rootChatId: String, title: String, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
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
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        PageBar(title, onBack)
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
            group("outline", null) {
                nodes.forEachIndexed { i, node ->
                    if (i > 0) RowDivider()
                    val chat = node.chatId
                    ListRow(
                        if (node.kind == TreeNodeKind.ROOT) title else node.title,
                        listOfNotNull(node.word, node.time, node.tokens).joinToString(" · "),
                        glyph = glyph(node.status),
                        tint = if (node.muted) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                        strong = node.depth == 0u,
                        indent = (node.depth.toInt() * 14).dp,
                        chevron = chat != null,
                        onClick = chat?.let { { onOpenChat(it) } },
                        trailing = { BrandMarks(listOfNotNull(node.brand)) },
                    )
                }
            }
        }
    }
}
