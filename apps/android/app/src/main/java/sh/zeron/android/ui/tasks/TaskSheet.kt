package sh.zeron.android.ui.tasks

import android.graphics.BitmapFactory
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.TasksText
import sh.zeron.android.core.lastDraftConfig
import sh.zeron.android.core.taskHostId
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.Staging
import sh.zeron.android.ui.StagedImage
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.MissionOptions
import uniffi.zeron_core.TaskDetail
import uniffi.zeron_core.TaskDraft
import uniffi.zeron_core.TaskStatus
import uniffi.zeron_core.missionLabels

private const val MAX_IMAGES = 12

private suspend fun upload(client: CoreClient, device: String, images: List<StagedImage>): List<String> =
    images.map { client.uploadAttachment(device, it.name, it.bytes, null) }

private fun draft(text: String, images: List<String>): TaskDraft {
    val lines = text.trim().split("\n", limit = 2)
    return TaskDraft(lines.first(), lines.getOrNull(1)?.trim().orEmpty(), null, images, emptyList())
}

private sealed interface Confirm {
    data class Image(val path: String, val title: String) : Confirm
    data class Context(val chat: String, val title: String) : Confirm
    data class Assign(val chat: String) : Confirm
    data object Delete : Confirm
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TaskSheet(model: AppModel, taskId: String, onDismiss: () -> Unit, onOpenThread: (String) -> Unit, onOpenMission: (String) -> Unit) {
    val client by model.client.collectAsState()
    val ws by model.workspace.collectAsState()
    val detail = remember(client, ws) { client?.taskDetail(taskId) }
    var seen by remember { mutableStateOf(false) }
    LaunchedEffect(detail) {
        if (detail != null) seen = true else if (seen) onDismiss()
    }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    var confirm by remember { mutableStateOf<Confirm?>(null) }
    var picking by remember { mutableStateOf<Boolean?>(null) }
    val thumbs = remember { mutableStateMapOf<String, ImageBitmap>() }
    fun run(action: suspend (CoreClient) -> Unit) {
        val c = client ?: return
        scope.launch { runCatching { action(c) }.onFailure { model.toast(TasksText.error(it)) } }
    }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_IMAGES)) { uris ->
        val d = detail ?: return@rememberLauncherForActivityResult
        val device = d.row.host?.deviceId ?: model.taskHostId() ?: return@rememberLauncherForActivityResult
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        run { c ->
            val staged = uris.take(MAX_IMAGES - d.images.size).mapNotNull { Staging.stage(context, it) }
            val paths = upload(c, device, staged)
            c.setTaskImages(taskId, (d.images + paths).take(MAX_IMAGES))
        }
    }
    LaunchedEffect(detail?.images, detail?.row?.host?.deviceId) {
        val d = detail ?: return@LaunchedEffect
        val device = d.row.host?.deviceId ?: return@LaunchedEffect
        val c = client ?: return@LaunchedEffect
        for (path in d.images) {
            if (thumbs.containsKey(path)) continue
            val bytes = runCatching { c.readAttachment(device, path) }.getOrNull() ?: continue
            withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }?.let { thumbs[path] = it }
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState()) {
        val d = detail
        if (d == null) {
            Text("This task is gone.", Modifier.padding(24.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
            return@ModalBottomSheet
        }
        val task = d.row
        val labels = remember { missionLabels() }
        LazyColumn(Modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 24.dp)) {
            item { Text("Task", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(start = 32.dp, bottom = 12.dp)) }
            group("about", null) {
                ListRow(task.title, task.notes.ifEmpty { null })
                RowDivider()
                ListRow(task.word, "Status", icon = ZIcons.Sort)
                task.dueOn?.let {
                    RowDivider()
                    ListRow(TasksText.dueLabel(it), "Due", icon = ZIcons.Bell)
                }
                RowDivider()
                ListRow(task.project?.name ?: "No project", "Project", icon = ZIcons.Folder)
                task.host?.let {
                    RowDivider()
                    ListRow(it.name, if (it.online) "Runs on" else "Runs on · offline", icon = ZIcons.Laptop)
                }
            }
            group("images", "Images · ${d.images.size}") {
                d.images.forEachIndexed { i, path ->
                    if (i > 0) RowDivider()
                    ImageRow(i, path, thumbs[path]) { confirm = Confirm.Image(path, "Image ${i + 1}") }
                }
                if (d.images.size < MAX_IMAGES) {
                    if (d.images.isNotEmpty()) RowDivider()
                    ListRow("Add images", icon = ZIcons.Plus, tint = MaterialTheme.colorScheme.primary, onClick = {
                        pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    })
                }
            }
            if (!task.mission || d.context.isNotEmpty()) {
                group("context", "Context chats · ${d.context.size}") {
                    d.context.forEachIndexed { i, chat ->
                        if (i > 0) RowDivider()
                        ListRow(chat.title, icon = ZIcons.Chat, onClick = { confirm = Confirm.Context(chat.chatId, chat.title) })
                    }
                    if (d.context.size < 5 && d.plan.isEmpty()) {
                        if (d.context.isNotEmpty()) RowDivider()
                        ListRow("Add a context chat", icon = ZIcons.Plus, tint = MaterialTheme.colorScheme.primary, onClick = { picking = false })
                    }
                }
            }
            group("actions", null) {
                val accent = MaterialTheme.colorScheme.primary
                var first = true
                @Composable fun action(title: String, icon: Int, tint: androidx.compose.ui.graphics.Color, onClick: () -> Unit) {
                    if (!first) RowDivider()
                    first = false
                    ListRow(title, icon = icon, tint = tint, onClick = onClick)
                }
                val text = MaterialTheme.colorScheme.onSurface
                if (task.mission) action(labels.openMissionPage, ZIcons.TabTasks, accent) { onOpenMission(taskId) }
                val lead = task.leadChatId
                if (lead != null) {
                    action("Open thread", ZIcons.Chat, accent) { onOpenThread(lead) }
                    if (!task.mission && task.status != TaskStatus.DONE) action("Unassign", ZIcons.Close, text) { run { it.unassignTask(taskId) } }
                } else if (task.status != TaskStatus.DONE) {
                    action("Start in a new chat", ZIcons.Send, accent) { start(model, d, null, ::run, onOpenThread, onOpenMission) }
                    action("Hand to a thread", ZIcons.ArrowRight, accent) { picking = true }
                    action(labels.planFirst, ZIcons.TabTasks, accent) {
                        start(model, d, MissionOptions(approvePlan = true, sameAgents = false, planDoc = true), ::run, onOpenThread, onOpenMission)
                    }
                }
                if (task.status == TaskStatus.DONE) {
                    action("Mark as not done", ZIcons.Restart, text) { run { it.setTaskCompleted(taskId, false) } }
                } else {
                    action("Mark as done", ZIcons.Check, text) { run { it.setTaskCompleted(taskId, true) } }
                }
                action("Delete", ZIcons.Delete, MaterialTheme.colorScheme.error) { confirm = Confirm.Delete }
            }
        }
    }
    picking?.let { assign ->
        val device = detail?.row?.host?.deviceId
        val taken = detail?.context?.map { it.chatId }.orEmpty().toSet()
        ThreadPicker(
            model,
            if (assign) "Hand to a thread" else "Context chat",
            include = { row -> assign || (row.id !in taken && (device == null || row.deviceId == device)) },
            onDismiss = { picking = null },
        ) { chat ->
            picking = null
            if (assign) {
                confirm = Confirm.Assign(chat)
            } else {
                run { it.setTaskContext(taskId, detail?.context.orEmpty().map { c -> c.chatId } + chat) }
            }
        }
    }
    when (val c = confirm) {
        null -> Unit
        is Confirm.Image -> ChoiceDialog(c.title, null, listOf("Remove" to true), onDismiss = { confirm = null }) {
            confirm = null
            run { cl -> cl.setTaskImages(taskId, detail?.images.orEmpty().filter { it != c.path }) }
        }
        is Confirm.Context -> {
            val choices = buildList {
                add("Open thread" to false)
                if (detail?.plan.isNullOrEmpty()) add("Remove" to true)
            }
            ChoiceDialog(c.title, null, choices, onDismiss = { confirm = null }) { choice ->
                confirm = null
                if (choice == "Open thread") onOpenThread(c.chat) else run { cl ->
                    cl.setTaskContext(taskId, detail?.context.orEmpty().map { it.chatId }.filter { it != c.chat })
                }
            }
        }
        is Confirm.Assign -> {
            val name = model.row(c.chat)?.title ?: "this thread"
            ChoiceDialog("Hand to $name", null, listOf("Send the task" to false, "Link only" to false), onDismiss = { confirm = null }) { choice ->
                confirm = null
                run { it.assignTask(taskId, c.chat, choice == "Send the task") }
            }
        }
        Confirm.Delete -> ChoiceDialog("Delete this task?", null, listOf("Delete" to true), onDismiss = { confirm = null }) {
            confirm = null
            run { it.deleteTask(taskId) }
        }
    }
}

private fun start(
    model: AppModel,
    detail: TaskDetail,
    mission: MissionOptions?,
    run: (suspend (CoreClient) -> Unit) -> Unit,
    onOpenThread: (String) -> Unit,
    onOpenMission: (String) -> Unit,
) {
    val device = detail.row.host?.deviceId ?: model.taskHostId()
    if (device == null) {
        model.toast("No Mac is online.")
        return
    }
    val id = detail.row.id
    run { c ->
        val chat = c.startTask(id, device, model.lastDraftConfig(), mission)
        if (mission == null) onOpenThread(chat) else onOpenMission(id)
    }
}

@Composable
private fun ImageRow(index: Int, path: String, thumb: ImageBitmap?, onClick: () -> Unit) {
    ListRow(
        "Image ${index + 1}",
        path.substringAfterLast('/'),
        icon = if (thumb == null) ZIcons.Image else null,
        onClick = onClick,
        trailing = thumb?.let {
            {
                Image(it, null, Modifier.size(44.dp).clip(RoundedCornerShape(6.dp)), contentScale = ContentScale.Crop)
            }
        },
    )
}

@Composable
fun ChoiceDialog(title: String, message: String?, choices: List<Pair<String, Boolean>>, onDismiss: () -> Unit, onChoose: (String) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = message?.let { { Text(it) } },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                for ((label, destructive) in choices) {
                    TextButton(onClick = { onChoose(label) }) {
                        Text(label, color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary)
                    }
                }
                TextButton(onClick = onDismiss) { Text("Cancel") }
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ThreadPicker(model: AppModel, title: String, include: (uniffi.zeron_core.SessionRow) -> Boolean, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    val client by model.client.collectAsState()
    val ws by model.workspace.collectAsState()
    var query by remember { mutableStateOf("") }
    val rows = remember(query, ws, client) {
        val q = query.trim()
        val all = if (q.isEmpty()) {
            val front = ws?.front
            (front?.pinned.orEmpty() + front?.recent.orEmpty() + front?.sections.orEmpty().flatMap { it.sessions }).distinctBy { it.id }
        } else {
            client?.search(q, 60u)?.map { it.session }.orEmpty()
        }
        all.filter(include)
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(0.92f).imePadding()) {
            Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 24.dp))
            OutlinedTextField(
                query,
                { query = it },
                placeholder = { Text("Threads") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
                shape = RoundedCornerShape(28.dp),
            )
            LazyColumn(Modifier.fillMaxWidth().weight(1f)) {
                items(rows, key = { it.id }) { row ->
                    ListRow(
                        row.title,
                        listOfNotNull(row.project?.name, row.deviceName, row.timeLabel).joinToString(" · "),
                        onClick = { onPick(row.id) },
                        trailing = { BrandMarks(row.harnessMarks) },
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewTaskSheet(model: AppModel, planFirst: Boolean, onDismiss: () -> Unit, onCreated: (taskId: String, mission: Boolean) -> Unit) {
    val ws by model.workspace.collectAsState()
    val client by model.client.collectAsState()
    val hosts = ws?.devices.orEmpty().filter { it.isExecutionHost }
    var hostId by remember { mutableStateOf(model.taskHostId()) }
    var title by remember { mutableStateOf("") }
    var notes by remember { mutableStateOf("") }
    var mission by remember { mutableStateOf(planFirst) }
    var approve by remember { mutableStateOf(true) }
    var images by remember { mutableStateOf<List<StagedImage>>(emptyList()) }
    var busy by remember { mutableStateOf(false) }
    var hostMenu by remember { mutableStateOf(false) }
    val labels = remember { missionLabels() }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_IMAGES)) { uris ->
        scope.launch { images = images + uris.take(MAX_IMAGES - images.size).mapNotNull { Staging.stage(context, it) } }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp).navigationBarsPadding().imePadding().padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onDismiss) { Text("Cancel") }
                Text("New task", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                Button(
                    enabled = !busy && title.isNotBlank() && hostId != null,
                    onClick = {
                        val c = client ?: return@Button
                        val device = hostId ?: return@Button
                        busy = true
                        scope.launch {
                            runCatching {
                                val paths = upload(c, device, images)
                                val d = draft("$title\n$notes", paths)
                                if (mission) {
                                    c.startMission(device, d, model.lastDraftConfig(), MissionOptions(approvePlan = approve, sameAgents = false, planDoc = true))
                                } else {
                                    c.createTask(device, d)
                                }
                            }.onSuccess { onCreated(it, mission) }.onFailure {
                                busy = false
                                model.toast(TasksText.error(it))
                            }
                        }
                    },
                ) { Text("Create") }
            }
            OutlinedTextField(title, { title = it }, placeholder = { Text("Title") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(notes, { notes = it }, placeholder = { Text("Notes") }, modifier = Modifier.fillMaxWidth().heightIn(min = 120.dp))
            Box {
                FilledTonalButton(onClick = { hostMenu = true }, modifier = Modifier.fillMaxWidth()) {
                    Text("Runs on " + (hosts.firstOrNull { it.id == hostId }?.name ?: "no Mac"))
                }
                DropdownMenu(hostMenu, onDismissRequest = { hostMenu = false }) {
                    for (h in hosts) {
                        DropdownMenuItem(
                            text = { Text(if (h.online) h.name else "${h.name} · offline") },
                            trailingIcon = if (h.id == hostId) ({ sh.zeron.android.design.ZIcon(ZIcons.Check, null, Modifier.size(18.dp)) }) else null,
                            onClick = {
                                hostId = h.id
                                hostMenu = false
                            },
                        )
                    }
                }
            }
            FilledTonalButton(onClick = { pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }, modifier = Modifier.fillMaxWidth()) {
                Text(if (images.isEmpty()) "Add images" else "${images.size} image${if (images.size == 1) "" else "s"} · add more")
            }
            SwitchRow(labels.planFirst, mission) { mission = it }
            if (mission) SwitchRow(labels.reviewPlan, approve) { approve = it }
        }
    }
}

@Composable
fun SwitchRow(label: String, on: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(on, onChange, enabled = enabled)
    }
}
