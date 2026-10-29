package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ThemeMode
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.ListItemShapes
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import sh.zeron.android.core.WallpaperStore
import uniffi.zeron_core.ChatMissionLabels
import uniffi.zeron_core.chatMissionLabels
import uniffi.zeron_core.coreVersion
import uniffi.zeron_core.whatsNew
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsScreen(
    model: AppModel,
    onArchived: () -> Unit,
    onOpenVoiceSettings: () -> Unit = {},
    onBack: (() -> Unit)? = null,
    onOpenProviders: () -> Unit = {},
    onOpenCloud: () -> Unit = {},
    onOpenBilling: () -> Unit = {},
    onOpenRoute: (String) -> Unit = {},
) {
    val appearance by model.appearance.collectAsState()
    val groupByProject by model.groupByProject.collectAsState()
    var confirmSignOut by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(false) }
    val workspace by model.workspace.collectAsState()
    val devices = workspace?.devices.orEmpty()
    val client by model.client.collectAsState()
    LaunchedEffect(client) { client?.let { model.wallpaper.refresh(it) } }
    val mission = remember { chatMissionLabels() }
    val latest = remember { whatsNew() }
    SettingsPage("Settings", onBack) {
        item {
            // Account: a tonal hero card with a shaped monogram.
            Surface(
                shape = RoundedCornerShape(32.dp),
                color = MaterialTheme.colorScheme.primaryContainer,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            ) {
                Row(Modifier.padding(20.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(
                        Modifier.size(64.dp).clip(MaterialShapes.Cookie9Sided.toShape()).background(MaterialTheme.colorScheme.primary),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(model.accountName.take(1).uppercase(), style = MaterialTheme.typography.headlineSmallEmphasized, color = MaterialTheme.colorScheme.onPrimary)
                    }
                    Spacer(Modifier.width(16.dp))
                    Column {
                        Text(model.accountName, style = MaterialTheme.typography.titleLargeEmphasized, color = MaterialTheme.colorScheme.onPrimaryContainer)
                        Text(model.accountDetail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f))
                    }
                }
            }
        }
        // Agents and where they run, as the desktop groups them: Providers,
        // then Cloud and Billing when this account is offered Cloud.
        section("Agents")
        item {
            val cloud = model.cloud
            LaunchedEffect(Unit) { cloud.refresh() }
            val offered = model.cloudOffered
            val count = if (offered) 3 else 1
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                CloudRow(0, count, "Providers", "Agents and their accounts on each device", leading = { IconTile(ZIcons.Bot) }, onClick = onOpenProviders)
                if (offered) {
                    CloudRow(1, count, "Cloud", cloud.problem?.title ?: cloud.githubAccount?.let { "GitHub @$it" } ?: "Connect GitHub", leading = { IconTile(ZIcons.Cloud) }, onClick = onOpenCloud)
                    val credits = cloud.credits?.let { "${sh.zeron.android.core.CloudStore.credits(it)} credits" }
                    val plan = cloud.billing?.plan?.let { "${it.name} plan" }
                    CloudRow(2, count, "Billing", listOfNotNull(plan, credits).joinToString(" · ").ifEmpty { "Plans and credits" }, leading = { IconTile(ZIcons.Wallet) }, onClick = onOpenBilling)
                }
            }
        }
        section("Appearance")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                Surface(shape = segmentedShapes(0, 2).shape, color = cardColor()) {
                    Column(Modifier.padding(16.dp)) {
                        Text("Theme", style = MaterialTheme.typography.titleMedium)
                        Spacer(Modifier.height(12.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
                            val modes = listOf(
                                Triple(ThemeMode.System, "System", ZIcons.Monitor),
                                Triple(ThemeMode.Light, "Light", ZIcons.Sun),
                                Triple(ThemeMode.Dark, "Dark", ZIcons.Moon),
                            )
                            modes.forEachIndexed { index, (mode, label, icon) ->
                                ToggleButton(
                                    checked = appearance.mode == mode,
                                    onCheckedChange = { model.setAppearance(appearance.copy(mode = mode)) },
                                    modifier = Modifier.weight(1f).semantics { role = Role.RadioButton },
                                    shapes = when (index) {
                                        0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                                        modes.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                                        else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                                    },
                                ) {
                                    ZIcon(icon, null, Modifier.size(ToggleButtonDefaults.IconSize))
                                    Spacer(Modifier.size(ToggleButtonDefaults.IconSpacing))
                                    Text(label)
                                }
                            }
                        }
                    }
                }
                SegmentedListItem(
                    onClick = { model.setAppearance(appearance.copy(dynamicColor = !appearance.dynamicColor)) },
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Magic) },
                    supportingContent = { Text("Tint Zeron with your wallpaper's palette") },
                    trailingContent = { Switch(appearance.dynamicColor, { model.setAppearance(appearance.copy(dynamicColor = it)) }) },
                ) { Text("Wallpaper colors") }
            }
        }
        section("Wallpaper")
        item { WallpaperSettings(model) }
        section("Sessions")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                SegmentedListItem(
                    onClick = { model.setGroupByProject(!groupByProject) },
                    shapes = segmentedShapes(0, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Project) },
                    supportingContent = { Text("Recent sessions grouped by repository") },
                    trailingContent = { Switch(groupByProject, { model.setGroupByProject(it) }) },
                ) { Text("Group by project") }
                SegmentedListItem(
                    onClick = onArchived,
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Archive) },
                    supportingContent = { (workspace?.archived?.size ?: 0).takeIf { it > 0 }?.let { Text("$it sessions") } },
                    trailingContent = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) },
                ) { Text("Archived sessions") }
            }
        }
        item {
            // Voice: once any registered host can run it (iOS More → Voice).
            if (sh.zeron.android.voice.rememberVoiceVisible(model.voice)) {
                val style by model.voice.selectedStyle.collectAsState()
                Column {
                    Text(
                        "Voice",
                        style = MaterialTheme.typography.titleSmallEmphasized,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 28.dp, top = 24.dp, bottom = 8.dp),
                    )
                    SegmentedListItem(
                        onClick = onOpenVoiceSettings,
                        shapes = segmentedShapes(0, 1),
                        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                        leadingContent = { IconTile(sh.zeron.android.R.drawable.zi_microphone) },
                        supportingContent = {
                            Text(style?.let { "Codex voice · ${sh.zeron.android.voice.VoiceController.styleLabel(it)}" } ?: "Codex voice device and style")
                        },
                        modifier = Modifier.padding(horizontal = 16.dp),
                    ) { Text("Voice") }
                }
            }
        }
        if (devices.isNotEmpty()) {
            section("Devices")
            item {
                Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                    devices.forEachIndexed { i, device ->
                        val opens = device.isExecutionHost && !device.isSelf
                        SegmentedListItem(
                            onClick = { if (opens) onOpenRoute("page:mac@${device.name}") },
                            shapes = segmentedShapes(i, devices.size),
                            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                            leadingContent = {
                                IconTile(
                                    when {
                                        !device.isExecutionHost -> ZIcons.Phone
                                        device.platform == "linux" -> ZIcons.Server
                                        else -> ZIcons.Laptop
                                    },
                                )
                            },
                            supportingContent = {
                                Text(
                                    listOfNotNull(
                                        if (device.isSelf) "This device" else if (device.online) "Online" else "Offline",
                                        device.version?.let { "v$it" },
                                        if (device.sessionCount > 0u) "${device.sessionCount} sessions" else null,
                                    ).joinToString(" · "),
                                )
                            },
                            trailingContent = {
                                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                    Box(
                                        Modifier.size(10.dp).clip(CircleShape).background(
                                            if (device.online || device.isSelf) successColor() else MaterialTheme.colorScheme.outlineVariant,
                                        ),
                                    )
                                    if (opens) ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline)
                                }
                            },
                        ) { Text(device.name) }
                    }
                }
            }
        }
        section("Composer")
        item { ComposerSettings(model) }
        section(mission.title)
        item { ChatMissionSettings(mission) }
        section("Help")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                SegmentedListItem(
                    onClick = { onOpenRoute("whatsnew") },
                    shapes = segmentedShapes(0, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Magic) },
                    supportingContent = latest?.let { { Text("v${it.version} · ${it.date}") } },
                    trailingContent = { Chevron() },
                ) { Text("What's New") }
                SegmentedListItem(
                    onClick = { onOpenRoute("docs") },
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.File) },
                    supportingContent = { Text("How Zeron works, by topic") },
                    trailingContent = { Chevron() },
                ) { Text("Docs") }
            }
        }
        section("About")
        item {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                SegmentedListItem(
                    onClick = {},
                    shapes = segmentedShapes(0, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Info) },
                    supportingContent = { Text("Zeron for Android · core ${coreVersion()}") },
                ) { Text("Version") }
                SegmentedListItem(
                    onClick = { if (model.isDemo) model.signOut() else confirmSignOut = true },
                    shapes = segmentedShapes(1, 2),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Logout, MaterialTheme.colorScheme.errorContainer, MaterialTheme.colorScheme.onErrorContainer) },
                ) { Text(if (model.isDemo) "Leave demo" else "Sign out", color = MaterialTheme.colorScheme.error) }
            }
        }
    }
    if (confirmSignOut) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            icon = { ZIcon(ZIcons.Logout, null) },
            title = { Text("Sign out?") },
            text = { Text("Messages still waiting to send and the New session draft are deleted. Text left in a session's composer stays on this device.") },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = {
                    confirmSignOut = false
                    model.signOut()
                }) { Text("Sign out", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { androidx.compose.material3.TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") } },
        )
    }
}

private fun LazyListScope.section(title: String) = settingsSection(title)

fun LazyListScope.settingsSection(title: String) {
    item {
        Text(
            title,
            style = MaterialTheme.typography.titleSmallEmphasized,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 28.dp, top = 24.dp, bottom = 8.dp),
        )
    }
}

fun LazyListScope.settingsFooter(text: String) {
    item {
        Text(
            text,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 28.dp, end = 28.dp, top = 8.dp),
        )
    }
}

/**
 * A settings page: a large title that collapses into the bar as the list
 * scrolls. While `busy`, a spinner shows under the bar and the list takes no taps.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsPage(
    title: String,
    onBack: (() -> Unit)?,
    busy: Boolean = false,
    state: LazyListState = rememberLazyListState(),
    content: LazyListScope.() -> Unit,
) {
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    Scaffold(
        modifier = Modifier.fillMaxSize().nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    if (onBack != null) {
                        TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, modifier = Modifier.padding(start = 8.dp), size = 44.dp, container = MaterialTheme.colorScheme.surfaceContainerHighest)
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.background,
                ),
                scrollBehavior = scroll,
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
            LazyColumn(Modifier.fillMaxSize(), state = state, contentPadding = PaddingValues(bottom = 48.dp), content = content)
            if (busy) {
                Box(
                    Modifier
                        .matchParentSize()
                        .pointerInput(Unit) {
                            awaitPointerEventScope {
                                while (true) awaitPointerEvent(PointerEventPass.Initial).changes.forEach { it.consume() }
                            }
                        },
                )
                LoadingIndicator(Modifier.align(Alignment.TopCenter).padding(top = 24.dp))
            }
        }
    }
}

@Composable
private fun CheckMark(on: Boolean) {
    if (on) ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
}

/** Follow-ups sent while an agent runs: queued, or steering the run (the composer's default). */
@Composable
private fun ComposerSettings(model: AppModel) {
    val steer by model.steerByDefault.collectAsState()
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
        SegmentedListItem(
            onClick = { model.setSteerByDefault(false) },
            shapes = segmentedShapes(0, 2),
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.Queue) },
            supportingContent = { Text("Queue follow-ups while the agent runs") },
            trailingContent = { CheckMark(!steer) },
        ) { Text("Queue") }
        SegmentedListItem(
            onClick = { model.setSteerByDefault(true) },
            shapes = segmentedShapes(1, 2),
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.ArrowRight) },
            supportingContent = { Text("Steer the current run") },
            trailingContent = { CheckMark(steer) },
        ) { Text("Steer") }
    }
}

/** Mission Control in a chat: the chat leads its mission, or starts a new one (read by the composer). */
@Composable
private fun ChatMissionSettings(labels: ChatMissionLabels) {
    val prefs = LocalContext.current.getSharedPreferences("settings", 0)
    var lead by remember { mutableStateOf(prefs.getString("chatMission", "lead") != "newMission") }
    fun pick(on: Boolean) {
        lead = on
        prefs.edit().putString("chatMission", if (on) "lead" else "newMission").apply()
    }
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
        SegmentedListItem(
            onClick = { pick(true) },
            shapes = segmentedShapes(0, 2),
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.TabTasks) },
            supportingContent = { Text(labels.detail) },
            trailingContent = { CheckMark(lead) },
        ) { Text(labels.lead) }
        SegmentedListItem(
            onClick = { pick(false) },
            shapes = segmentedShapes(1, 2),
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.Plus) },
            trailingContent = { CheckMark(!lead) },
        ) { Text(labels.newMission) }
    }
}

/** A leading glyph on a tonal rounded tile. */
@Composable
fun IconTile(
    icon: Int,
    container: Color = MaterialTheme.colorScheme.secondaryContainer,
    content: Color = MaterialTheme.colorScheme.onSecondaryContainer,
) {
    Box(Modifier.size(40.dp).clip(RoundedCornerShape(14.dp)).background(container), contentAlignment = Alignment.Center) {
        ZIcon(icon, null, Modifier.size(22.dp), tint = content)
    }
}

/** Wallpaper: shown behind new sessions, the sessions list and (optionally) open chats; or followed from a Mac. */
@Composable
private fun WallpaperSettings(model: AppModel) {
    val store = model.wallpaper
    val state by store.state.collectAsState()
    val client by model.client.collectAsState()
    val workspace by model.workspace.collectAsState()
    val hosts = remember(client, workspace) { client?.wallpaperHosts().orEmpty() }
    val host = hosts.firstOrNull { it.id == store.syncDeviceId } ?: hosts.firstOrNull()
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val picker = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia(),
    ) { uri -> if (uri != null) scope.launch { store.set(uri, "Photo") } }
    var effects by remember { mutableStateOf(false) }
    var macs by remember { mutableStateOf(false) }
    val syncing = state.syncWithMac
    val chat = state.chat
    val rows = mutableListOf<@Composable (ListItemShapes) -> Unit>()
    rows += { shapes ->
        val follows = host?.let { if (syncing) "Follows ${it.name}${if (it.online) "" else " (offline)"}" else "Use ${it.name}'s background and chat settings" }
        val enabled = host != null || syncing
        SegmentedListItem(
            onClick = {
                store.setSyncWithMac(!syncing)
                client?.let { c -> scope.launch { store.refresh(c) } }
            },
            enabled = enabled,
            shapes = shapes,
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.Laptop) },
            supportingContent = { Text(follows ?: "No Mac in this workspace") },
            trailingContent = {
                Switch(syncing, {
                    store.setSyncWithMac(it)
                    client?.let { c -> scope.launch { store.refresh(c) } }
                }, enabled = enabled)
            },
        ) { Text("Sync with Mac") }
    }
    if (syncing && hosts.size > 1) {
        rows += { shapes ->
            Box {
                SegmentedListItem(
                    onClick = { macs = true },
                    shapes = shapes,
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Monitor) },
                    supportingContent = { host?.let { Text(it.name) } },
                    trailingContent = { Chevron() },
                ) { Text("Mac") }
                ChoiceMenu(macs, { macs = false }, listOf(MenuSection("Sync with Mac", hosts.map { h ->
                    MenuChoice(h.name, h.id == host?.id) {
                        store.setSyncDevice(h.id)
                        client?.let { c -> scope.launch { store.refresh(c) } }
                    }
                })))
            }
        }
    }
    rows += { shapes ->
        SegmentedListItem(
            onClick = {
                picker.launch(androidx.activity.result.PickVisualMediaRequest(androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia.ImageOnly))
            },
            enabled = !syncing,
            shapes = shapes,
            colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
            leadingContent = { IconTile(ZIcons.Image) },
            supportingContent = {
                Text(
                    when {
                        state.set -> state.name ?: "Photo"
                        syncing -> "None on the Mac"
                        else -> "Shown behind new sessions and the sessions list"
                    },
                )
            },
            trailingContent = { if (!syncing) Chevron() },
        ) { Text(if (syncing) "Wallpaper" else if (state.set) "Change wallpaper" else "Choose wallpaper") }
    }
    if (state.set) {
        rows += { shapes ->
            Box {
                SegmentedListItem(
                    onClick = { effects = true },
                    enabled = !syncing,
                    shapes = shapes,
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Magic) },
                    supportingContent = { Text("${WallpaperStore.label(state.effect)} — ${WallpaperStore.detail(state.effect)}") },
                    trailingContent = { if (!syncing) Chevron() },
                ) { Text("Effect") }
                ChoiceMenu(effects, { effects = false }, listOf(MenuSection("Effect", WallpaperStore.effects.map { e ->
                    MenuChoice(WallpaperStore.label(e), e == state.effect) { store.setEffect(e) }
                })))
            }
        }
        rows += { shapes ->
            SegmentedListItem(
                onClick = { store.setChat(chat.copy(enabled = !chat.enabled)) },
                enabled = !syncing,
                shapes = shapes,
                colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                leadingContent = { IconTile(ZIcons.Chat) },
                supportingContent = { Text("Show the artwork behind open conversations.") },
                trailingContent = { Switch(chat.enabled, { store.setChat(chat.copy(enabled = it)) }, enabled = !syncing) },
            ) { Text("Chat background") }
        }
        if (chat.enabled) {
            rows += { shapes ->
                SegmentedListItem(
                    onClick = {},
                    enabled = !syncing,
                    shapes = shapes,
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Monitor) },
                    supportingContent = { Text(chat.placement.detail) },
                    trailingContent = {
                        Row(horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
                            WallpaperStore.Placement.entries.forEachIndexed { index, p ->
                                ToggleButton(
                                    checked = chat.placement == p,
                                    onCheckedChange = { store.setChat(chat.copy(placement = p)) },
                                    enabled = !syncing,
                                    modifier = Modifier.semantics { role = Role.RadioButton },
                                    shapes = if (index == 0) ButtonGroupDefaults.connectedLeadingButtonShapes() else ButtonGroupDefaults.connectedTrailingButtonShapes(),
                                ) { Text(p.label) }
                            }
                        }
                    },
                ) { Text("Placement") }
            }
            rows += { shapes ->
                SliderRow(shapes, "Opacity", ZIcons.Sun, chat.opacity, 0f..1f, 99, "${(chat.opacity * 100).roundToInt()}%", !syncing) {
                    store.setChat(chat.copy(opacity = (it * 100).roundToInt() / 100f))
                }
            }
            rows += { shapes ->
                val max = WallpaperStore.ChatBackground.BLUR_MAX
                val step = WallpaperStore.ChatBackground.BLUR_STEP
                SliderRow(shapes, "Blur", ZIcons.Effort, chat.blur, 0f..max, (max / step).toInt() - 1, "${chat.blur.roundToInt()}px", !syncing) {
                    store.setChat(chat.copy(blur = it))
                }
            }
            rows += { shapes ->
                SegmentedListItem(
                    onClick = { store.setChat(chat.copy(scrim = !chat.scrim)) },
                    enabled = !syncing,
                    shapes = shapes,
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Moon) },
                    supportingContent = { Text("Darkens the artwork toward the bottom so text stays readable.") },
                    trailingContent = { Switch(chat.scrim, { store.setChat(chat.copy(scrim = it)) }, enabled = !syncing) },
                ) { Text("Scrim") }
            }
        }
        if (!syncing) {
            rows += { shapes ->
                SegmentedListItem(
                    onClick = { store.remove() },
                    shapes = shapes,
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { IconTile(ZIcons.Delete, MaterialTheme.colorScheme.errorContainer, MaterialTheme.colorScheme.onErrorContainer) },
                ) { Text("Remove wallpaper", color = MaterialTheme.colorScheme.error) }
            }
        }
    }
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
        rows.forEachIndexed { i, row -> row(segmentedShapes(i, rows.size)) }
    }
}

@Composable
private fun SliderRow(
    shapes: ListItemShapes,
    title: String,
    icon: Int,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    steps: Int,
    label: String,
    enabled: Boolean,
    onChange: (Float) -> Unit,
) {
    SegmentedListItem(
        onClick = {},
        enabled = enabled,
        shapes = shapes,
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        leadingContent = { IconTile(icon) },
        supportingContent = { Slider(value, onChange, enabled = enabled, valueRange = range, steps = steps) },
        trailingContent = { Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) },
    ) { Text(title) }
}
