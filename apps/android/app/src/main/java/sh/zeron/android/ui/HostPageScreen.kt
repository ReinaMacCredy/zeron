package sh.zeron.android.ui

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel

@Composable
fun HostPageScreen(model: AppModel, page: String, host: String?, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage(host ?: "Mac", onBack)
}
