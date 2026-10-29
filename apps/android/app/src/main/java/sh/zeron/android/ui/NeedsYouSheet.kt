package sh.zeron.android.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.AppModel

@Composable
fun NeedsYouPill(model: AppModel, chatId: String, onClick: () -> Unit) {
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NeedsYouSheet(model: AppModel, excluding: String?, onDismiss: () -> Unit, onOpenChat: (String) -> Unit, onOpenRoute: (String) -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Text("Needs you", style = MaterialTheme.typography.titleLarge, modifier = Modifier.fillMaxWidth().padding(24.dp))
    }
}
