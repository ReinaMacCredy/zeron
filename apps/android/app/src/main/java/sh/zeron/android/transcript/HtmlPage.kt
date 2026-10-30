package sh.zeron.android.transcript

import android.annotation.SuppressLint
import android.content.Context
import android.net.Uri
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import org.json.JSONObject
import sh.zeron.android.R
import sh.zeron.android.design.Geist
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.TranscriptPalette
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.HtmlSheet
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.WidgetKind
import uniffi.zeron_core.htmlRenderDocument
import java.util.WeakHashMap

class HtmlPageState(private val source: suspend () -> String?) {
    var html by mutableStateOf<String?>(null)
        private set
    var failed by mutableStateOf(false)
        private set
    var attempt by mutableIntStateOf(0)
        private set
    internal var web by mutableStateOf<PageWebView?>(null)
    internal var crashes = 0
    var onHeight: (Float) -> Unit = {}
    var openUrl: (String) -> Unit = {}

    internal suspend fun load() {
        if (html != null) return
        failed = false
        val loaded = source()
        if (loaded == null) failed = true else html = loaded
    }

    fun retry() {
        attempt++
    }

    internal fun webView(context: Context): PageWebView =
        web ?: PageWebView(context, this).also { web = it }

    internal fun crashed() {
        web?.destroy()
        web = null
        crashes++
    }

    fun destroy() {
        web?.destroy()
        web = null
    }
}

private const val MAX_PAGES = 6
private val pools = WeakHashMap<TranscriptState, LinkedHashMap<String, HtmlPageState>>()

private fun pooledPage(state: TranscriptState, id: String, source: suspend () -> String?): HtmlPageState {
    val pool = pools.getOrPut(state) { LinkedHashMap(16, 0.75f, true) }
    pool[id]?.let { return it }
    val page = HtmlPageState(source)
    pool[id] = page
    if (pool.size > MAX_PAGES) {
        pool.entries.firstOrNull { it.key != id && it.value.web?.parent == null }?.let {
            pool.remove(it.key)
            it.value.destroy()
        }
    }
    return page
}

@Composable
fun HtmlPageWidget(state: TranscriptState, kind: WidgetKind.HtmlRender, actions: TranscriptActions) {
    val page = remember(state, kind.id) { pooledPage(state, kind.id) { actions.loadHtml(kind.deviceId, kind.path) } }
    page.onHeight = { state.engine.setHtmlHeight(kind.id, it) }
    page.openUrl = actions.openUrl
    HtmlPage(page, kind.scrolls)
}

@Composable
fun HtmlPage(page: HtmlPageState, scrolls: Boolean, modifier: Modifier = Modifier) {
    val dark = LocalDarkTheme.current
    val scale = LocalDensity.current.fontScale
    LaunchedEffect(page, page.attempt) { page.load() }
    Box(modifier.fillMaxSize()) {
        val html = page.html
        if (html == null) {
            PageStatus(page.failed, page::retry)
            return@Box
        }
        if (page.web == null && page.crashes > 2) return@Box
        key(page.web) {
            AndroidView(
                factory = { context ->
                    page.webView(context).also { (it.parent as? ViewGroup)?.removeView(it) }
                },
                update = { web ->
                    web.scrolls = scrolls
                    web.render(html, dark, scale)
                },
                modifier = Modifier.fillMaxSize().semantics { contentDescription = "HTML page" },
            )
        }
    }
}

@Composable
private fun PageStatus(failed: Boolean, onRetry: () -> Unit) {
    Box(
        Modifier.fillMaxSize().clip(RoundedCornerShape(10.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (!failed) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.outline)
            }
            Text(
                if (failed) "Couldn't load this page." else "Loading page",
                fontFamily = Geist,
                fontSize = 13.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (failed) {
                FilledTonalButton(onClick = onRetry, contentPadding = PaddingValues(horizontal = 14.dp)) {
                    ZIcon(ZIcons.Refresh, null, Modifier.size(14.dp))
                    Text("Retry", fontFamily = Geist, fontSize = 14.sp, modifier = Modifier.padding(start = 6.dp))
                }
            }
        }
    }
}

@Composable
fun HtmlExpandButton(kind: WidgetKind.HtmlExpand, actions: TranscriptActions, palette: TranscriptPalette) {
    var open by remember { mutableStateOf(false) }
    Box(
        Modifier
            .fillMaxSize()
            .clip(RoundedCornerShape(6.dp))
            .semantics { contentDescription = "Expand page" }
            .clickable { open = true },
        contentAlignment = Alignment.Center,
    ) {
        ZIcon(R.drawable.zi_tool_expand_arrows, null, Modifier.size(13.dp), tint = Color(palette[ColorRole.TEXT_TERTIARY]))
    }
    if (open) {
        HtmlSheet(kind.title, { actions.loadHtml(kind.deviceId, kind.path) }, actions.openUrl) { open = false }
    }
}

@Composable
fun rememberHtmlPage(source: suspend () -> String?): HtmlPageState {
    val page = remember { HtmlPageState(source) }
    DisposableEffect(page) { onDispose { page.destroy() } }
    return page
}

@SuppressLint("SetJavaScriptEnabled", "ViewConstructor")
internal class PageWebView(context: Context, private val page: HtmlPageState) : WebView(context) {
    var scrolls = false
        set(value) {
            field = value
            isVerticalScrollBarEnabled = value
        }
    private var rendered: Pair<Boolean, Float>? = null

    init {
        setBackgroundColor(android.graphics.Color.TRANSPARENT)
        isVerticalScrollBarEnabled = false
        isHorizontalScrollBarEnabled = false
        overScrollMode = OVER_SCROLL_NEVER
        settings.apply {
            javaScriptEnabled = true
            javaScriptCanOpenWindowsAutomatically = false
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            mediaPlaybackRequiresUserGesture = true
            textZoom = 100
            setGeolocationEnabled(false)
            setSupportZoom(false)
        }
        addJavascriptInterface(Bridge(), BRIDGE)
        webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val url = request.url
                if (!request.isForMainFrame) return url.scheme == "file"
                if (url.toString().startsWith("about:blank#")) return false
                if (request.hasGesture() && leavesForBrowser(url)) page.openUrl(url.toString())
                return true
            }

            override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
                (parent as? ViewGroup)?.removeView(this@PageWebView)
                page.crashed()
                return true
            }
        }
        webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) = request.deny()
        }
    }

    fun render(html: String, dark: Boolean, scale: Float) {
        val key = dark to scale
        if (rendered == key) return
        rendered = key
        // html_render_document wires the height bridge to WKWebView's message handler only.
        val document = htmlRenderDocument(html, dark, scale).replace(IOS_BRIDGE, "$BRIDGE.postMessage(String(m))")
        loadDataWithBaseURL(null, document, "text/html", "utf-8", null)
    }

    private inner class Bridge {
        @JavascriptInterface
        fun postMessage(message: String) {
            val height = runCatching { JSONObject(message).getDouble("height") }.getOrNull() ?: return
            post { page.onHeight(height.toFloat()) }
        }
    }

    private companion object {
        const val BRIDGE = "zeronAndroid"
        const val IOS_BRIDGE = "window.webkit.messageHandlers.zeron.postMessage(m)"

        fun leavesForBrowser(url: Uri): Boolean =
            (url.scheme == "http" || url.scheme == "https") && !url.host.isNullOrEmpty() && url.userInfo == null
    }
}
