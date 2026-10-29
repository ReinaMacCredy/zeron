package sh.zeron.android.ui.tasks

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel
import sh.zeron.android.ui.StubPage

@Composable
fun TasksScreen(
    model: AppModel,
    onOpenChat: (String) -> Unit,
    onOpenRoute: (String) -> Unit,
    openTask: String? = null,
    onBack: (() -> Unit)? = null,
) {
    StubPage("Tasks", onBack)
}

@Composable
fun MissionScreen(model: AppModel, taskId: String, onBack: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("Mission", onBack)
}

@Composable
fun SupervisorScreen(model: AppModel, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    StubPage("Supervisor", onBack)
}

@Composable
fun rememberTasksBadge(model: AppModel): Int = 0
