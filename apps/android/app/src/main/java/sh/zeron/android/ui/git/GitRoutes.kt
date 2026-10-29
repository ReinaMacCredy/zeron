package sh.zeron.android.ui.git

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel
import sh.zeron.android.ui.StubPage

@Composable
fun PullRequestsScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("Pull requests", onBack)
}

@Composable
fun PullRequestScreen(model: AppModel, url: String, deviceId: String?, onBack: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("Pull request", onBack)
}

@Composable
fun GitScreen(model: AppModel, history: Boolean, onBack: () -> Unit, onOpenChat: (String) -> Unit) {
    StubPage(if (history) "History" else "Git", onBack)
}
