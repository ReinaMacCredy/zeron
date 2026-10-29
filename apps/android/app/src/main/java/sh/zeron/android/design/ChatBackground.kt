package sh.zeron.android.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import sh.zeron.android.core.AppModel

@Composable
fun ChatBackground(model: AppModel, chatId: String, modifier: Modifier = Modifier) {
    val store = model.wallpaper
    val state by store.state.collectAsState()
    val client by model.client.collectAsState()
    LaunchedEffect(client, state.syncWithMac) { client?.let { store.refresh(it) } }
    if (!state.chat.visible) return
    WallpaperHero(store, modifier, chat = state.chat)
}
