package sh.zeron.android.ui

import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.transcript.TranscriptState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.focus.FocusRequester
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import uniffi.zeron_core.QueueEditAction
import uniffi.zeron_core.QueueEditLease
import uniffi.zeron_core.QueueEditStart
import java.util.UUID
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import uniffi.zeron_core.BusyPolicy
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.ComposerState
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.ModelInfo
import uniffi.zeron_core.QueueGate
import uniffi.zeron_core.QueueItem
import uniffi.zeron_core.SandboxLevel
import uniffi.zeron_core.SendRequest
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.fallbackModels
import uniffi.zeron_core.reasoningLabel
import androidx.compose.foundation.background
import androidx.compose.runtime.Stable
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.luminance
import sh.zeron.android.R
import sh.zeron.android.core.ComposerExtras
import uniffi.zeron_core.AccessMode
import uniffi.zeron_core.ModelSwitchPrompt
import uniffi.zeron_core.ProfileRow
import uniffi.zeron_core.ProviderInstanceRow
import uniffi.zeron_core.QueuePrimaryAction
import uniffi.zeron_core.WorkspaceCommand
import uniffi.zeron_core.accessModes
import uniffi.zeron_core.compactCommand
import uniffi.zeron_core.contextCard
import uniffi.zeron_core.fastModeOn
import uniffi.zeron_core.followUpNotice
import uniffi.zeron_core.isClaudeCardCommand
import uniffi.zeron_core.missionComposerLabels
import uniffi.zeron_core.missionStagedError
import uniffi.zeron_core.missionText
import uniffi.zeron_core.opensConfigPanel
import uniffi.zeron_core.pickContext
import uniffi.zeron_core.profileMenuLabels
import uniffi.zeron_core.resolveAccess
import uniffi.zeron_core.resumePrompt
import uniffi.zeron_core.sidechatCommand
import uniffi.zeron_core.speedToggle
import uniffi.zeron_core.toggleLongContext
import uniffi.zeron_core.toggleSpeed
import uniffi.zeron_core.typedWorkspaceCommand

private enum class Delivery(val label: String) { Queue("Queue"), Steer("Steer"), Interrupt("Stop & send") }

@Composable
fun Composer(
    model: AppModel,
    client: CoreClient,
    handle: SessionHandle,
    c: ComposerState,
    row: SessionRow?,
    transcript: TranscriptState,
    draft: ComposerModel,
    beforeSend: () -> Boolean = { true },
    openModelPicker: Boolean = false,
    onOpenClaudeConfig: () -> Unit = {},
    onNeedsYou: () -> Unit = {},
    onOpenRoute: (String) -> Unit = {},
    onCompact: (String?) -> Unit = {},
    onSidechat: (String?) -> Unit = {},
    onOpenFindings: () -> Unit = {},
    quotes: kotlinx.coroutines.flow.Flow<String> = kotlinx.coroutines.flow.emptyFlow(),
) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    // The model picker lives here, not in the chips: those leave with the expanded card.
    var picker by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        if (openModelPicker) {
            kotlinx.coroutines.delay(600)
            picker = true
        }
    }
    val switches = remember(c.chatId) { ConfigSwitches(client, c.chatId) }
    switches.Prompt()
    SessionModelPicker(client, c, row, picker, switches, onDismiss = { picker = false })
    val focus = remember { FocusRequester() }
    var error by remember { mutableStateOf<String?>(null) }
    val steerDefault by model.steerByDefault.collectAsState()
    val canSteer = c.host.capabilities.midTurnSteering == true
    var delivery by remember(steerDefault) { mutableStateOf(if (steerDefault) Delivery.Steer else Delivery.Queue) }
    val running = c.live.turnRunning
    val harness = row?.harness ?: "claude-code"
    val deviceId = c.host.deviceId
    var slashCatalog by remember(c.chatId) { mutableStateOf<uniffi.zeron_core.InvocationCatalog?>(null) }
    var renaming by remember { mutableStateOf(false) }
    val missionAvailable = row?.sidechatOf == null
    val mission = remember(c.chatId) { MissionDraft() }
    if (!missionAvailable) mission.on = false

    LaunchedEffect(quotes) {
        quotes.collect {
            draft.quotes.add(it)
            focus.requestFocus()
        }
    }

    // Queue editing: the row being edited, its lease, and the user's own draft parked meanwhile.
    var editingId by remember { mutableStateOf<String?>(null) }
    var lease by remember { mutableStateOf<QueueEditLease?>(null) }
    var editBase by remember { mutableStateOf<Pair<String, String>?>(null) }
    var renewal by remember { mutableStateOf<Job?>(null) }
    var stash by remember { mutableStateOf<Pair<String, List<StagedImage>>?>(null) }
    var editPending by remember { mutableStateOf(false) }

    fun restoreStash() {
        stash?.let { (text, images) ->
            draft.clear()
            draft.setText(text)
            draft.images.addAll(images)
        }
        stash = null
    }

    fun finishEdit(text: String?) {
        val held = lease ?: return
        val base = editBase
        renewal?.cancel()
        lease = null
        editBase = null
        editingId = null
        // Unchanged: release the row as it was (a commit would drop what the composer never showed).
        val body = if (text != null && base != null) {
            if (text == base.second) null else replacingVisible(base.first, base.second, text)
        } else {
            text
        }
        scope.launch { runCatching { handle.finishQueuedEdit(held, if (body == null) QueueEditAction.CANCEL else QueueEditAction.COMMIT, body) } }
    }

    fun beginEdit(id: String) {
        if (editPending) return
        editPending = true
        scope.launch {
            try {
                if (editingId != null) {
                    finishEdit(null)
                    restoreStash()
                }
                val start = runCatching { handle.beginQueuedEdit(id, UUID.randomUUID().toString()) }.getOrNull()
                if (start !is QueueEditStart.Acquired) {
                    error = "Can't edit right now — another device is editing it, or it was just sent."
                    return@launch
                }
                lease = start.lease
                renewal = scope.launch {
                    while (true) {
                        delay(20_000)
                        val held = lease ?: break
                        if (!runCatching { handle.renewQueuedEdit(held) }.getOrDefault(false)) break
                    }
                }
                val item = handle.composer().queue.firstOrNull { it.id == id }
                editBase = item?.let { it.text to it.visibleText }
                if (stash == null) stash = draft.text to draft.images.toList()
                draft.clear()
                draft.setText(item?.visibleText ?: "")
                editingId = id
                error = null
                focus.requestFocus()
            } finally {
                editPending = false
            }
        }
    }

    fun runCommand(command: WorkspaceCommand) {
        when (command) {
            WorkspaceCommand.MODEL -> picker = true
            WorkspaceCommand.NEW -> onOpenRoute("new")
            WorkspaceCommand.RESUME -> onOpenRoute("search")
            WorkspaceCommand.SETTINGS -> onOpenRoute("settings")
            WorkspaceCommand.DIFF -> scope.launch {
                if (runCatching { handle.chatChanges() }.getOrNull() == null) model.toast("No changes in this chat yet") else onOpenRoute("git")
            }
            WorkspaceCommand.FILES -> {
                draft.beginFileMention()
                focus.requestFocus()
            }
            WorkspaceCommand.TERMINAL -> onOpenRoute("chat:${c.chatId}/terminal")
            WorkspaceCommand.RENAME -> renaming = true
            WorkspaceCommand.STOP -> runCatching { handle.interrupt() }
        }
    }

    fun sent() {
        draft.clear()
        model.drafts.save(c.chatId, "")
        error = null
        delivery = if (steerDefault) Delivery.Steer else Delivery.Queue
    }

    fun startMission() {
        val pictures = draft.images.toList()
        mission.error(pictures.size)?.let {
            error = it
            return
        }
        val brief = missionText(draft.encoded(), draft.pasted.map { it.text })
        val options = mission.options()
        scope.launch {
            try {
                val paths = pictures.map { client.uploadAttachment(deviceId, it.name, it.bytes, null) }
                val started = client.startMissionFromChat(c.chatId, missionDraft(brief, null, paths), options, ComposerExtras.missionLead(context))
                started.notice?.let { model.toast(it) }
                sent()
                mission.on = false
                onOpenRoute("tasks:mission:${started.taskId}")
            } catch (e: Exception) {
                error = e.message ?: "Couldn't start the mission"
            }
        }
    }

    fun send(mode: Delivery = delivery) {
        if (!draft.hasContent) return
        if (editingId != null) {
            finishEdit(draft.encoded())
            restoreStash()
            return
        }
        slashCatalog?.let { typedWorkspaceCommand(it, draft.text) }?.let {
            sent()
            return runCommand(it)
        }
        if (mission.on) return startMission()
        val body = draft.encoded()
        val claude = harness == "claude-code"
        val skills = ComposerExtras.skillNames(deviceId, harness)
        if (claude && isClaudeCardCommand(body, skills)) {
            sent()
            scope.launch {
                try {
                    client.runClaudeCard(c.chatId, body, skills)?.let { switches.apply(it, row) }
                } catch (e: Exception) {
                    model.toast(e.message ?: "Couldn't run $body")
                }
            }
            return
        }
        if (claude && opensConfigPanel(body)) {
            sent()
            return onOpenClaudeConfig()
        }
        compactCommand(body)?.let {
            sent()
            return onCompact(it.focus)
        }
        if (row?.sidechatOf == null) sidechatCommand(body)?.let { command ->
            sent()
            return onSidechat(command.prompt)
        }
        if (!beforeSend()) return
        try {
            val queued = running && mode == Delivery.Queue
            if (mode == Delivery.Interrupt && running) handle.interrupt()
            handle.send(SendRequest(body, draft.attachments, null, if (running && mode == Delivery.Steer && canSteer) BusyPolicy.STEER else BusyPolicy.QUEUE))
            model.noteDeliveryActivity()
            sent()
            // An immediate send gets the runway; one queued behind a live turn
            // takes it over once its bubble lands.
            if (queued) transcript.expectQueuedTurn() else transcript.beginOwnTurn()
        } catch (e: Exception) {
            error = "Couldn't send: ${e.message}"
        }
    }

    // The user's own draft (never a queued message being edited) is kept
    // across navigation, the question panel, and the app going away.
    fun saveDraft() = model.drafts.save(c.chatId, stash?.first ?: draft.text)
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
    DisposableEffect(c.chatId, lifecycle) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_STOP) saveDraft()
        }
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            saveDraft()
            // Leaving the session gives an edited row straight back.
            if (editingId != null) {
                finishEdit(null)
                restoreStash()
            }
        }
    }

    c.queueError?.let { queueError ->
        LaunchedEffect(queueError) {
            model.toast(queueError)
            runCatching { handle.clearQueueError() }
        }
    }
    if (renaming) RenameDialog(row?.title ?: c.title, onDismiss = { renaming = false }) { model.rename(c.chatId, it) }

    val queueing = running && draft.hasContent && editingId == null
    Column(Modifier.padding(horizontal = 12.dp).padding(bottom = 8.dp)) {
        if (c.queue.isNotEmpty()) QueuePanel(c.queue, handle, editingId, onEdit = ::beginEdit, image = { path -> runCatching { client.readAttachment(deviceId, path) }.getOrNull() })
        sh.zeron.android.ui.composer.ComposerSuggestions(
            draft,
            files = { q -> client.searchFiles(deviceId, c.chatId, null, q) },
            threads = { q -> client.threadMentions(deviceId, row?.project?.id, c.chatId, q) },
            catalog = { skill ->
                ComposerExtras.invocationCatalog(client, deviceId, harness, c.chatId, null, skill).also { if (!skill) slashCatalog = it }
            },
            onInvocation = { q, entry ->
                when {
                    draft.withoutQuery(q).isBlank() && harness == "claude-code" && opensConfigPanel(entry.insert) -> {
                        draft.setText("")
                        onOpenClaudeConfig()
                    }
                    entry.workspaceCommand != null -> {
                        draft.dropQuery(q)
                        runCommand(entry.workspaceCommand!!)
                    }
                    else -> draft.insertInvocation(q, entry)
                }
            },
        )
        error?.let {
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(start = 16.dp, bottom = 6.dp))
        }
        when {
            editingId != null -> StatusBanner("Editing queued message", "Cancel" to {
                finishEdit(null)
                restoreStash()
            })
            else -> followUpNotice(queueing, delivery == Delivery.Steer, canSteer)?.let { StatusBanner(it, null) }
        }
        val labels = remember { missionComposerLabels(true) }
        ComposerSurface(
            model = draft,
            placeholder = when {
                editingId != null -> "Edit queued message"
                mission.on -> labels.placeholder
                else -> "Message ${row?.harnessLabel ?: "the agent"}"
            },
            action = when {
                editingId != null -> ComposerAction.Send
                running && !draft.hasContent -> ComposerAction.Stop
                running -> ComposerAction.Queue
                c.live.resumable && !draft.hasContent -> ComposerAction.Resume
                else -> ComposerAction.Send
            },
            onAction = {
                when {
                    editingId == null && running && !draft.hasContent -> runCatching { handle.interrupt() }
                    editingId == null && !running && c.live.resumable && !draft.hasContent ->
                        runCatching { handle.send(SendRequest(resumePrompt(), emptyList(), null, BusyPolicy.QUEUE)) }
                    else -> send()
                }
            },
            attach = editingId == null,
            focusRequester = focus,
            prediction = if (editingId == null && !mission.on) c.prediction else null,
        ) {
            if (running && editingId == null) DeliveryChip(delivery, canSteer) { delivery = it }
            if (missionAvailable && editingId == null) MissionChips(mission, labels)
            SessionChips(client, handle, c, row, switches, onOpenPicker = { picker = true }, onOpenFindings = onOpenFindings)
        }
    }
}

@Stable
class MissionDraft {
    var on by mutableStateOf(false)
    var review by mutableStateOf(true)
    var planDoc by mutableStateOf(true)

    fun options() = uniffi.zeron_core.MissionOptions(review, false, planDoc)

    fun error(images: Int): String? = missionStagedError(images.toUInt(), null)
}

fun missionDraft(text: String, spaceId: String?, images: List<String>): uniffi.zeron_core.TaskDraft {
    val trimmed = text.trim()
    val title = trimmed.substringBefore('\n')
    val notes = if (trimmed.contains('\n')) trimmed.substringAfter('\n').trim() else ""
    return uniffi.zeron_core.TaskDraft(title, notes, spaceId, images, emptyList())
}

@Composable
fun MissionChips(mission: MissionDraft, labels: uniffi.zeron_core.MissionComposerLabels) {
    ContextChip(
        labels.start,
        leading = { ZIcon(R.drawable.zi_hugeicons_target_02, null, Modifier.size(16.dp)) },
        onClick = { mission.on = !mission.on },
        tint = if (mission.on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
    )
    if (mission.on) {
        var open by remember { mutableStateOf(false) }
        ContextChip(
            if (mission.planDoc) labels.planDoc else labels.stepsOnly,
            leading = { ZIcon(if (mission.review) R.drawable.zi_checklist else R.drawable.zi_document, null, Modifier.size(16.dp)) },
            onClick = { open = true },
        ) {
            ChoiceMenu(
                open,
                { open = false },
                listOf(
                    MenuSection(labels.start, listOf(
                        MenuChoice(labels.planDoc, mission.planDoc) { mission.planDoc = true },
                        MenuChoice(labels.stepsOnly, !mission.planDoc) { mission.planDoc = false },
                    )),
                    MenuSection(null, listOf(MenuChoice(labels.reviewPlanFirst, mission.review) { mission.review = !mission.review })),
                ),
            )
        }
    }
}

/** The row's raw text with its visible part replaced, keeping the hidden context after it. */
private fun replacingVisible(raw: String, visible: String, edited: String): String {
    if (edited.isBlank() || raw == visible) return edited
    val body = raw.trimStart()
    if (body.startsWith(visible)) return edited + body.substring(visible.length)
    // Attachment-only rows show a placeholder: all of the raw text is context.
    return if (body.isEmpty()) edited else edited + "\n\n" + body
}

@Composable
private fun DeliveryChip(current: Delivery, canSteer: Boolean, onChange: (Delivery) -> Unit) {
    var open by remember { mutableStateOf(false) }
    ContextChip(
        current.label,
        leading = { ZIcon(ZIcons.Queue, null, Modifier.size(16.dp)) },
        onClick = { open = true },
        tint = MaterialTheme.colorScheme.primary,
    ) {
        ChoiceMenu(
            open,
            { open = false },
            listOf(
                MenuSection(
                    "While the agent works",
                    listOfNotNull(
                        MenuChoice("Queue", current == Delivery.Queue, "Send when this turn ends") { onChange(Delivery.Queue) },
                        if (canSteer) MenuChoice("Steer", current == Delivery.Steer, "Add to the running turn") { onChange(Delivery.Steer) } else null,
                        MenuChoice("Stop & send", current == Delivery.Interrupt, "Stop the turn, then send") { onChange(Delivery.Interrupt) },
                    ),
                ),
            ),
        )
    }
}

@Stable
class ConfigSwitches(private val client: CoreClient, private val chatId: String) {
    private var pending by mutableStateOf<Pair<ModelSwitchPrompt, suspend () -> Unit>?>(null)
    private var failure by mutableStateOf<String?>(null)
    var profiles by mutableStateOf<List<ProfileRow>>(emptyList())
    var version by mutableStateOf(0)
        private set

    fun current(harness: String): ChatConfig =
        client.sessionConfig(chatId) ?: ChatConfig(harness, null, null, emptyMap(), SandboxLevel.WORKSPACE_WRITE)

    fun set(harness: String, change: (ChatConfig) -> ChatConfig) {
        runCatching { client.setSessionConfig(chatId, change(current(harness))) }.onFailure { failure = it.message }
        version++
    }

    fun confirm(prompt: ModelSwitchPrompt?, harness: String, change: (ChatConfig) -> ChatConfig) {
        if (prompt == null) return set(harness, change)
        pending = prompt to { client.setSessionConfigNoted(chatId, change(current(harness)), prompt.note, prompt.noteHint) }
    }

    fun pickModel(harness: String, id: String, fromLabel: String, toLabel: String) =
        confirm(client.modelSwitchPrompt(chatId, id, fromLabel, toLabel), harness) { it.copy(model = id) }

    fun apply(write: uniffi.zeron_core.ChipWrite, row: SessionRow?) {
        val harness = row?.harness ?: "claude-code"
        write.model?.let { id -> pickModel(harness, id, row?.modelLabel ?: current(harness).model ?: "", write.modelLabel ?: id) }
        write.reasoning?.let { r -> set(harness) { it.copy(reasoning = r) } }
        write.permission?.let { p -> set(harness) { it.copy(access = p) } }
    }

    suspend fun loadProfiles() {
        profiles = runCatching { client.profileRows(chatId) }.getOrDefault(emptyList())
    }

    suspend fun applyProfile(id: String) {
        try {
            val prompt = client.profileSwitchPrompt(chatId, id)
            if (prompt == null) client.applyAgentProfile(chatId, id, null, null)
            else pending = prompt to { client.applyAgentProfile(chatId, id, prompt.note, prompt.noteHint) }
        } catch (e: Exception) {
            failure = e.message
        }
        loadProfiles()
    }

    suspend fun saveProfile(id: String) {
        runCatching { client.saveChatIntoProfile(chatId, id) }.onFailure { failure = it.message }
        loadProfiles()
    }

    suspend fun detachProfile() {
        runCatching { client.detachAgentProfile(chatId) }.onFailure { failure = it.message }
        loadProfiles()
    }

    fun fail(message: String) {
        failure = message
    }

    @Composable
    fun Prompt() {
        val scope = rememberCoroutineScope()
        pending?.let { (prompt, run) ->
            androidx.compose.material3.AlertDialog(
                onDismissRequest = { pending = null },
                title = { Text(prompt.title) },
                text = { Text("${prompt.lead}.\n\n${prompt.body}") },
                confirmButton = {
                    TextButton(onClick = {
                        pending = null
                        scope.launch {
                            runCatching { run() }.onFailure { failure = it.message }
                            version++
                        }
                    }) { Text(prompt.confirm) }
                },
                dismissButton = { TextButton(onClick = { pending = null }) { Text(prompt.cancel) } },
            )
        }
        failure?.let {
            androidx.compose.material3.AlertDialog(
                onDismissRequest = { failure = null },
                text = { Text(it) },
                confirmButton = { TextButton(onClick = { failure = null }) { Text("OK") } },
            )
        }
    }
}

// The transcript palette's WARNING role (iOS Palette.warning), for chips outside the transcript.
@Composable
fun warningTint(): androidx.compose.ui.graphics.Color =
    if (MaterialTheme.colorScheme.surface.luminance() < 0.5f) androidx.compose.ui.graphics.Color(0xFFFACC15) else androidx.compose.ui.graphics.Color(0xFFA16207)

@androidx.annotation.DrawableRes
fun accessIcon(mode: AccessMode): Int = when (mode) {
    AccessMode.SUPERVISED -> R.drawable.zi_checklist
    AccessMode.AUTO_ACCEPT_EDITS -> R.drawable.zi_magic_stick_3
    AccessMode.FULL_ACCESS -> R.drawable.zi_danger_triangle
}

@Composable
fun AccessChip(current: AccessMode, offered: List<AccessMode>?, onPick: (AccessMode) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val info = accessModes(null).firstOrNull { it.mode == current } ?: return
    ContextChip(
        info.label,
        leading = { ZIcon(accessIcon(current), null, Modifier.size(16.dp)) },
        onClick = { open = true },
        tint = if (current == AccessMode.FULL_ACCESS) warningTint() else MaterialTheme.colorScheme.onSurfaceVariant,
    ) {
        ChoiceMenu(open, { open = false }, listOf(MenuSection("Access", accessModes(offered).map { m ->
            MenuChoice(m.label, m.mode == current, m.description, leading = { ZIcon(accessIcon(m.mode), null, Modifier.size(20.dp)) }) { onPick(m.mode) }
        })))
    }
}

@Composable
private fun SessionChips(client: CoreClient, handle: SessionHandle, c: ComposerState, row: SessionRow?, switches: ConfigSwitches, onOpenPicker: () -> Unit, onOpenFindings: () -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val harness = row?.harness ?: "claude-code"
    var models by remember(c.host.deviceId, harness) { mutableStateOf<List<ModelInfo>?>(null) }
    var menu by remember { mutableStateOf<String?>(null) }
    var contextSheet by remember { mutableStateOf(false) }
    LaunchedEffect(c.host.deviceId, harness) {
        models = runCatching { client.listModels(c.host.deviceId, harness) }.getOrNull()?.takeIf { it.isNotEmpty() } ?: fallbackModels(harness)
    }
    val config = remember(c.revision, row?.revision, switches.version) { client.sessionConfig(c.chatId) }
    LaunchedEffect(config?.profileId, row?.revision) {
        if (config?.profileId != null || switches.profiles.isNotEmpty()) switches.loadProfiles()
    }
    val current = models?.let { list -> list.firstOrNull { it.id == row?.model } ?: list.firstOrNull() }

    // No model of its own: it runs the provider's default, so that's the one named.
    val fallback = remember(harness) { fallbackModels(harness).firstOrNull() }
    val modelLabel = row?.modelLabel ?: models?.firstOrNull()?.label ?: fallback?.label ?: row?.harnessLabel
    if (modelLabel != null) {
        val title = switches.profiles.firstOrNull { it.active }?.let { "${it.name}${if (it.drift == null) "" else " •"} · $modelLabel" } ?: modelLabel
        ContextChip(title, leading = { HarnessMark(harness, 14.dp) }, onClick = onOpenPicker)
    }
    row?.reasoning?.takeIf { it.isNotEmpty() }?.let { level ->
        val fast = current != null && config != null && fastModeOn(current, config)
        ContextChip(reasoningLabel(level), leading = { ZIcon(if (fast) R.drawable.zi_bolt else ZIcons.Effort, null, Modifier.size(16.dp)) }, onClick = { menu = "effort" }) {
            val speed = if (current != null && config != null) speedToggle(current, config) else null
            ChoiceMenu(menu == "effort", { menu = null }, listOfNotNull(
                MenuSection("Effort", current?.reasoningLevels.orEmpty().map { l ->
                    MenuChoice(reasoningLabel(l), l == level) { switches.set(harness) { it.copy(reasoning = l) } }
                }),
                speed?.let { on ->
                    MenuSection(null, listOf(MenuChoice("Fast", on, leading = { ZIcon(R.drawable.zi_bolt, null, Modifier.size(20.dp)) }) {
                        switches.set(harness) { toggleSpeed(current!!, it) }
                    }))
                },
            ))
        }
    }
    val offered = c.host.capabilities.accessModes
    AccessChip(resolveAccess(config?.access ?: AccessMode.FULL_ACCESS, offered), offered) { mode -> switches.set(harness) { it.copy(access = mode) } }
    val pr = row?.pullRequest
    if (pr != null) {
        val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
        ContextChip("#${pr.number}", leading = { ZIcon(ZIcons.PullRequest, null, Modifier.size(16.dp)) }, onClick = { menu = "pr" }) {
            ActionMenu(
                menu == "pr",
                { menu = null },
                listOf(
                    MenuAction("Open pull request", ZIcons.Link) { CustomTabsIntent.Builder().build().launchUrl(context, Uri.parse(pr.url)) },
                    MenuAction("Copy link", ZIcons.Copy) { clipboard.setText(androidx.compose.ui.text.AnnotatedString(pr.url)) },
                ),
            )
        }
    } else {
        row?.branch?.takeIf { it.isNotEmpty() }?.let {
            ContextChip(it, leading = { ZIcon(ZIcons.Branch, null, Modifier.size(16.dp)) }, onClick = {})
        }
    }
    val tokens = c.contextUsage?.tokens
    val window = c.contextUsage?.window
    if (tokens != null && window != null && window > 0u) {
        val fraction = tokens.toDouble() / window.toDouble()
        val warn = fraction >= 0.85
        ContextChip(
            "${Math.round(fraction * 100)}% context",
            leading = { ZIcon(if (warn) ZIcons.Warning else ZIcons.Context, null, Modifier.size(16.dp)) },
            onClick = { contextSheet = true },
            tint = if (warn) warningTint() else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    } else if (handle.statsView(System.currentTimeMillis()) != null || handle.compactSection(System.currentTimeMillis()) != null) {
        ContextChip("Context", leading = { ZIcon(ZIcons.Context, null, Modifier.size(16.dp)) }, onClick = { contextSheet = true })
    }
    c.review?.let { review -> ReviewChip(review, handle, onOpenFindings) }
    if (contextSheet) {
        sh.zeron.android.ui.composer.ContextSheet(
            handle,
            card = { tokensNow -> if (current != null) client.sessionConfig(c.chatId)?.let { contextCard(current, it, tokensNow) } else null },
            onToggleLong = { if (current != null) switches.set(harness) { toggleLongContext(current, it) } },
            onPick = { position -> if (current != null) switches.set(harness) { pickContext(current, it, position) } },
            onDismiss = { contextSheet = false },
        )
    }
}

@Composable
private fun ReviewChip(review: uniffi.zeron_core.ReviewChipView, handle: SessionHandle, onOpenFindings: () -> Unit) {
    val scope = rememberCoroutineScope()
    var error by remember { mutableStateOf<String?>(null) }
    var stopMenu by remember { mutableStateOf(false) }
    fun run(work: suspend () -> Unit) {
        scope.launch { runCatching { work() }.onFailure { error = it.message ?: "Couldn't reach the review" } }
    }
    val label = buildString {
        append(review.label)
        review.nextReviewer?.let { append(" · $it") }
        if (review.additions != null && review.deletions != null) append("  +${review.additions} \u2212${review.deletions}")
        review.note?.let { append(" · $it") }
    }
    ContextChip(
        label,
        leading = { ZIcon(if (review.refresh) ZIcons.Refresh else if (review.clean) ZIcons.Check else ZIcons.TabTasks, null, Modifier.size(16.dp)) },
        onClick = {
            when {
                review.busy && review.stoppable -> stopMenu = true
                review.action == uniffi.zeron_core.ReviewChipAction.OPEN_FINDINGS -> onOpenFindings()
                review.action == uniffi.zeron_core.ReviewChipAction.START -> run { handle.startReview() }
                else -> {}
            }
        },
        tint = when (review.tone) {
            uniffi.zeron_core.ReviewTone.MUTED -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f)
            uniffi.zeron_core.ReviewTone.DANGER -> MaterialTheme.colorScheme.error
            uniffi.zeron_core.ReviewTone.SUCCESS -> successColor()
            uniffi.zeron_core.ReviewTone.PLAIN -> MaterialTheme.colorScheme.onSurfaceVariant
        },
    ) {
        ActionMenu(stopMenu, { stopMenu = false }, listOf(MenuAction(review.stopLabel, ZIcons.Stop) { run { handle.stopReview() } }))
    }
    error?.let { message ->
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { error = null },
            text = { Text(message) },
            confirmButton = { TextButton(onClick = { error = null }) { Text("OK") } },
        )
    }
}

@Composable
fun StatusBanner(text: String, action: Pair<String, () -> Unit>?) {
    Surface(
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Row(Modifier.padding(start = 16.dp, end = if (action != null) 4.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(vertical = 10.dp).weight(1f, fill = false))
            action?.let { (label, run) -> TextButton(onClick = run) { Text(label) } }
        }
    }
}

@Composable
fun QueuePanel(queue: List<QueueItem>, handle: SessionHandle, editingId: String?, onEdit: (String) -> Unit, image: suspend (String) -> ByteArray? = { null }) {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    Column(Modifier.fillMaxWidth().padding(bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        queue.forEachIndexed { i, item ->
            val editing = item.id == editingId
            Surface(
                shape = segmentShape(i, queue.size, outer = 20.dp, inner = 6.dp),
                color = if (editing) MaterialTheme.colorScheme.secondaryContainer else composerContainer(),
                border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(Modifier.padding(start = 14.dp, end = 4.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    ZIcon(ZIcons.Queue, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(10.dp))
                    item.image?.let { path ->
                        val bitmap by androidx.compose.runtime.produceState<androidx.compose.ui.graphics.ImageBitmap?>(null, path) {
                            value = image(path)?.let { bytes -> android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }
                        }
                        Box(Modifier.size(26.dp).clip(RoundedCornerShape(6.dp)).background(chipContainer())) {
                            bitmap?.let { androidx.compose.foundation.Image(it, null, Modifier.matchParentSize(), contentScale = androidx.compose.ui.layout.ContentScale.Crop) }
                        }
                        Spacer(Modifier.width(8.dp))
                    }
                    Column(Modifier.weight(1f)) {
                        val gate = if (editing) "Editing" else gate(item)
                        Text(
                            gate?.let { "$it · ${item.visibleText.ifEmpty { "Attachment" }}" } ?: item.visibleText.ifEmpty { "Attachment" },
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        item.fromLabel?.let {
                            Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                        }
                    }
                    val action = item.primaryAction
                    IconButton(
                        onClick = {
                            haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                            scope.launch { runCatching { handle.deliverQueuedNow(item.id) } }
                        },
                        enabled = action != null && !editing,
                    ) {
                        ZIcon(
                            ZIcons.ArrowUp,
                            when (action) {
                                QueuePrimaryAction.STEER -> "Steer"
                                QueuePrimaryAction.SEND_NEXT -> "Send next"
                                QueuePrimaryAction.SEND_NOW -> "Send now"
                                null -> "Send"
                            },
                            Modifier.size(20.dp),
                        )
                    }
                    var menu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { menu = true }) { ZIcon(ZIcons.More, "More queue actions", Modifier.size(20.dp)) }
                        ActionMenu(
                            menu,
                            { menu = false },
                            listOfNotNull(
                                if (!editing && item.gate == null) MenuAction("Edit", ZIcons.Rename) { onEdit(item.id) } else null,
                                if (i > 0) MenuAction("Move Up", ZIcons.ChevronUp) { runCatching { handle.moveQueuedBy(item.id, -1) } } else null,
                                if (i < queue.size - 1) MenuAction("Move Down", ZIcons.ChevronDown) { runCatching { handle.moveQueuedBy(item.id, 1) } } else null,
                                MenuAction("Remove", ZIcons.Delete, destructive = true) { scope.launch { runCatching { handle.removeQueued(item.id) } } },
                            ),
                        )
                    }
                }
            }
        }
    }
}

private fun gate(item: QueueItem): String? = when (val g = item.gate) {
    is QueueGate.Editing -> if (g.mine) "Editing" else "Being edited"
    is QueueGate.ReviewRequired -> "Needs review"
    null -> if (item.actionPending) "Updating" else null
}

@Composable
private fun SessionModelPicker(client: CoreClient, c: ComposerState, row: SessionRow?, open: Boolean, switches: ConfigSwitches, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val harness = row?.harness ?: "claude-code"
    // Another provider only between turns, on a host that can switch (and not while a send is on its way).
    val sendPending = c.sendState == uniffi.zeron_core.SendState.SENDING || c.sendState == uniffi.zeron_core.SendState.QUEUED
    val hostSwitches = c.host.capabilities.providerSwitch
    val canSwitch = hostSwitches && !c.live.turnRunning && !sendPending
    val cloud = row?.isCloud == true
    var hostHarnesses by remember { mutableStateOf<List<uniffi.zeron_core.HarnessInfo>>(emptyList()) }
    var instances by remember { mutableStateOf<List<ProviderInstanceRow>>(emptyList()) }
    LaunchedEffect(c.host.deviceId, canSwitch, cloud) {
        if (canSwitch && !cloud && hostHarnesses.isEmpty()) hostHarnesses = runCatching { client.listHarnesses(c.host.deviceId) }.getOrNull().orEmpty()
        if (!cloud && instances.isEmpty()) instances = runCatching { client.listProviderInstances(c.host.deviceId) }.getOrNull().orEmpty()
    }
    LaunchedEffect(open) { if (open) switches.loadProfiles() }
    // No model of its own: it runs the provider's default, so that's the one named and ticked.
    val fallback = remember(harness) { fallbackModels(harness).firstOrNull() }
    val currentModel = row?.model ?: fallback?.id
    if (!open) return
    val config = client.sessionConfig(c.chatId)
    val currentInstance = config?.instanceId
    suspend fun rowsFor(h: String): List<ModelPickerRow> {
        // A host that doesn't answer gets the built-in catalog, as one that errors does.
        val listed = within(5_000) {
            if (cloud) runCatching { client.cloudModels(h) }.getOrNull() else runCatching { client.listModels(c.host.deviceId, h) }.getOrNull()
        }
        return (listed?.takeIf { it.isNotEmpty() } ?: fallbackModels(h)).map { ModelPickerRow(it.id, it.label, it.description) }
    }
    suspend fun rowsFor(instance: ProviderInstanceRow, force: Boolean): List<ModelPickerRow> {
        val catalog = client.listInstanceModels(c.host.deviceId, instance.harness, instance.id, force)
        catalog.error?.let { return listOf(ModelPickerRow(RETRY + instance.id, profileMenuLabels().retry, it)) }
        return catalog.models.map { ModelPickerRow(it.id, it.label, it.description) }
    }
    val label = instances.firstOrNull { it.id == currentInstance }?.label ?: row?.harnessLabel ?: uniffi.zeron_core.agentLoginRules(harness).name.ifEmpty { harness }
    val currentKey = currentInstance?.let { INSTANCE + it } ?: harness
    // A computer offers what it reports; Cloud runs the agents it has accounts for.
    val others = if (!canSwitch) emptyList() else if (cloud) {
        sh.zeron.android.core.CloudStore.agents.map { it.harness to it.name }
    } else {
        hostHarnesses.filter { it.offered }.map { it.id to it.label }
    }
    val current = instances.firstOrNull { it.id == currentInstance }
    val tabs = listOf(ModelPickerTab(currentKey, label) { if (current != null) rowsFor(current, false) else rowsFor(harness) }) +
        others.filter { it.first != currentKey }.map { (h, l) -> ModelPickerTab(h, l, "Your next message continues this chat with $l.") { rowsFor(h) } } +
        (if (canSwitch) instances.filter { it.id != currentInstance } else emptyList()).map { inst ->
            ModelPickerTab(INSTANCE + inst.id, inst.label, "Your next message continues this chat with ${inst.label}.") { rowsFor(inst, false) }
        }
    val note = when {
        canSwitch -> null
        c.live.turnRunning -> "You can switch provider when this turn ends."
        hostSwitches -> null
        else -> "This computer needs a newer Zeron to switch provider."
    }
    val fromModel = row?.modelLabel ?: row?.model ?: ""
    ModelPickerSheet(
        tabs,
        currentKey,
        currentModel,
        host = if (cloud) "Cloud" else c.host.name ?: "This computer",
        hostIcon = if (cloud) ZIcons.Cloud else ZIcons.Laptop,
        note = note,
        header = if (switches.profiles.isEmpty()) null else ({ ProfilesRow(switches) }),
        onPick = pick@{ key, picked ->
            if (picked.id.startsWith(RETRY)) {
                val inst = instances.firstOrNull { it.id == picked.id.removePrefix(RETRY) } ?: return@pick
                scope.launch { rowsFor(inst, true).singleOrNull { it.id.startsWith(RETRY) }?.let { switches.fail("${inst.label}: ${it.detail}") } }
                return@pick
            }
            if (key == currentKey) {
                return@pick switches.pickModel(harness, picked.id, fromModel, picked.label)
            }
            // Re-checked at the pick: a turn may have started since the sheet opened.
            if (!canSwitch) return@pick
            val inst = instances.firstOrNull { INSTANCE + it.id == key }
            val toHarness = inst?.harness ?: key
            val toLabel = inst?.label ?: tabs.firstOrNull { it.harness == key }?.label ?: key
            switches.confirm(client.providerSwitchPrompt(c.chatId, label, toLabel, fromModel, picked.label), harness) {
                // The old provider's effort and options don't carry over.
                val switched = if (toHarness != it.harness) it.copy(harness = toHarness, reasoning = null, modelOptions = emptyMap()) else it
                switched.copy(model = picked.id, instanceId = inst?.id)
            }
        },
        onDismiss = onDismiss,
    )
}

private const val INSTANCE = "instance:"
private const val RETRY = "retry:"

@Composable
private fun ProfilesRow(switches: ConfigSwitches) {
    val scope = rememberCoroutineScope()
    val labels = remember { profileMenuLabels() }
    val none = switches.profiles.none { it.active }
    Column(Modifier.padding(start = 24.dp, end = 16.dp, top = 14.dp)) {
        Text(labels.title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.size(6.dp))
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            switches.profiles.forEach { p ->
                var menu by remember { mutableStateOf(false) }
                val edited = p.active && p.drift != null
                ContextChip(
                    if (edited) "${p.name} · ${labels.edited}" else p.name,
                    leading = { ZIcon(if (p.active) ZIcons.Check else R.drawable.zi_lucide_user_round, null, Modifier.size(16.dp)) },
                    onClick = { if (edited) menu = true else if (!p.active) scope.launch { switches.applyProfile(p.id) } },
                    tint = if (p.active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                ) {
                    GroupedActionMenu(menu, { menu = false }, listOf(listOfNotNull(
                        MenuAction(labels.revert, R.drawable.zi_lucide_undo_2) { scope.launch { switches.applyProfile(p.id) } },
                        if (p.savable) MenuAction(labels.save, R.drawable.zi_floppy_disk) { scope.launch { switches.saveProfile(p.id) } } else null,
                    )), header = p.drift)
                }
            }
            ContextChip(
                labels.none,
                leading = { ZIcon(if (none) ZIcons.Check else ZIcons.Close, null, Modifier.size(16.dp)) },
                onClick = { if (!none) scope.launch { switches.detachProfile() } },
                tint = if (none) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
