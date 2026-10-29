package sh.zeron.android.ui.help

import androidx.compose.runtime.Composable
import sh.zeron.android.core.AppModel
import sh.zeron.android.ui.StubPage

@Composable
fun WhatsNewScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("What's New", onBack)
}

@Composable
fun DocsScreen(model: AppModel, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("Docs", onBack)
}

@Composable
fun DocScreen(model: AppModel, id: String, onBack: () -> Unit, onOpenRoute: (String) -> Unit) {
    StubPage("Docs", onBack)
}
