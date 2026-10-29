package sh.zeron.android.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcons
import sh.zeron.android.transcript.HtmlPage
import sh.zeron.android.transcript.rememberHtmlPage
import uniffi.zeron_core.htmlRenderFileName
import uniffi.zeron_core.htmlRenderSource

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HtmlSheet(title: String, source: suspend () -> String?, openUrl: (String) -> Unit, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        DialogSystemBars()
        val page = rememberHtmlPage(source)
        page.openUrl = { url ->
            onDismiss()
            openUrl(url)
        }
        val context = LocalContext.current
        val scope = rememberCoroutineScope()
        var selected by remember { mutableIntStateOf(0) }
        val html = page.html
        val text = remember(html) { html?.let(::htmlRenderSource) }
        val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("text/html")) { uri ->
            val content = html ?: return@rememberLauncherForActivityResult
            uri ?: return@rememberLauncherForActivityResult
            scope.launch(Dispatchers.IO) {
                runCatching { context.contentResolver.openOutputStream(uri)?.use { it.write(content.toByteArray()) } }
            }
        }
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    navigationIcon = { TonalCircleButton(ZIcons.Close, "Close", onClick = onDismiss, modifier = Modifier.padding(start = 8.dp), size = 44.dp) },
                    actions = {
                        TextButton(onClick = { save.launch(htmlRenderFileName(title)) }, enabled = html != null) { Text("Save .html") }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
                )
            },
            bottomBar = {
                Box(Modifier.fillMaxWidth().navigationBarsPadding().padding(vertical = 10.dp), contentAlignment = Alignment.Center) {
                    SingleChoiceSegmentedButtonRow(Modifier.widthIn(max = 280.dp)) {
                        listOf("Live", "Source").forEachIndexed { i, label ->
                            SegmentedButton(
                                selected = selected == i,
                                onClick = { selected = i },
                                shape = SegmentedButtonDefaults.itemShape(i, 2),
                            ) { Text(label) }
                        }
                    }
                }
            },
            containerColor = MaterialTheme.colorScheme.background,
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding)) {
                if (selected == 1 && text != null) {
                    SelectionContainer {
                        Text(
                            text,
                            fontFamily = GeistMono,
                            fontSize = 12.sp,
                            color = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 16.dp),
                        )
                    }
                } else {
                    HtmlPage(page, scrolls = true)
                }
            }
        }
    }
}
