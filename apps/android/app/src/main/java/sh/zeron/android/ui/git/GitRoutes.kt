package sh.zeron.android.ui.git

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.GitModel
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.ScreenHeader
import sh.zeron.android.ui.TonalCircleButton
import uniffi.zeron_core.GitCheckout
import uniffi.zeron_core.PrDevice
import uniffi.zeron_core.PrRepository

@Composable
fun rememberGitModel(): GitModel {
    val context = LocalContext.current
    return remember { GitModel(context.applicationContext) }
}

@Composable
fun PullRequestsScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    PullsAndGit(model, git = false, history = false, onBack = onBack, onOpenRoute = onOpenRoute)
}

@Composable
fun PullRequestScreen(model: AppModel, url: String, deviceId: String?, onBack: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    PullRequestPageScreen(model, url, deviceId, onBack, onOpenRoute)
}

@Composable
fun GitScreen(model: AppModel, history: Boolean, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    PullsAndGit(model, git = true, history = history, onBack = onBack, onOpenRoute = {})
}

private class CommitOpen(val checkout: GitCheckout, val sha: String, val short: String)

@Composable
private fun PullsAndGit(model: AppModel, git: Boolean, history: Boolean, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    val client by model.client.collectAsState()
    val workspace by model.workspace.collectAsState()
    val prefs = rememberGitModel()
    var onGit by rememberSaveable { mutableStateOf(git) }
    var devices by remember { mutableStateOf<List<PrDevice>>(emptyList()) }
    var deviceId by remember { mutableStateOf<String?>(null) }
    var repositories by remember { mutableStateOf<List<PrRepository>?>(null) }
    var repository by remember { mutableStateOf<String?>(null) }
    var refresh by remember { mutableIntStateOf(0) }
    var reloadDevices by remember { mutableIntStateOf(0) }
    var commit by remember { mutableStateOf<CommitOpen?>(null) }

    LaunchedEffect(client, reloadDevices) {
        val c = client ?: return@LaunchedEffect
        devices = c.pullRequestDevices()
        deviceId = prefs.pickDevice(devices) ?: workspace?.projects?.firstOrNull { it.gitDetected }?.deviceId
    }
    LaunchedEffect(client, deviceId) {
        repositories = null
        repository = null
        val c = client ?: return@LaunchedEffect
        val id = deviceId ?: return@LaunchedEffect
        prefs.device = id
        val repos = c.pullRequestRepositories(id)
        if (deviceId != id) return@LaunchedEffect
        repositories = repos
        repository = prefs.pickRepository(repos)
    }
    val projectId = prefs.projectFor(deviceId, repository, repositories.orEmpty(), workspace)

    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        Column(Modifier.fillMaxSize().statusBarsPadding()) {
            Row(Modifier.padding(start = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                TonalCircleButton(ZIcons.Back, "Back", onClick = onBack)
            }
            ScreenHeader(if (onGit) "Git" else "Pull requests", null) {
                TonalCircleButton(ZIcons.Refresh, "Refresh", onClick = {
                    if (!onGit && devices.isEmpty()) reloadDevices++ else refresh++
                })
            }
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Segments(listOf("Pull requests", "Git"), if (onGit) 1 else 0, { onGit = it == 1 })
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MenuPill(
                        devices.firstOrNull { it.id == deviceId }?.name ?: workspace?.devices?.firstOrNull { it.id == deviceId }?.name ?: "No Mac",
                        devices.map { d -> MenuChoice(d.name, if (d.online) null else "Offline", d.id == deviceId) { deviceId = d.id } },
                        leading = ZIcons.Laptop,
                        title = "Desktop devices",
                    )
                    if (!onGit) {
                        MenuPill(
                            repository?.let { "Repository: ${it.substringAfterLast('/')}" } ?: "No repository",
                            repositories.orEmpty().map { r ->
                                MenuChoice(r.repository, checked = r.repository == repository) {
                                    repository = r.repository
                                    prefs.repository = r.repository
                                }
                            },
                            title = "Repository",
                        )
                    }
                }
            }
            val c = client
            Box(Modifier.weight(1f)) {
                when {
                    c == null -> Unit
                    onGit -> {
                        val device = deviceId
                        if (device == null) {
                            GitNoticeList("No Mac serves pull requests", "Open Zeron on a Mac signed in to GitHub with gh, then refresh.")
                        } else {
                            GitPane(model, c, device, projectId, startHistory = history, refresh = refresh) { checkout, row ->
                                commit = CommitOpen(checkout, row.sha, row.shortSha)
                            }
                        }
                    }
                    else -> BoardPane(
                        client = c,
                        prefs = prefs,
                        devicesEmpty = devices.isEmpty(),
                        deviceId = deviceId,
                        repository = repository,
                        repositoriesLoaded = repositories != null,
                        refresh = refresh,
                        onOpen = { url ->
                            deviceId?.let { prefs.device = it }
                            onOpenRoute("pr:$url")
                        },
                    )
                }
            }
        }
        commit?.let { open ->
            BackHandler { commit = null }
            CommitDetail(model, open.checkout, open.sha, open.short, onBack = { commit = null })
        }
    }
}
