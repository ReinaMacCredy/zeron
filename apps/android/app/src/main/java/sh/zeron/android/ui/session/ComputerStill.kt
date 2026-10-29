package sh.zeron.android.ui.session

import android.graphics.BitmapFactory
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.withContext
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.ActionMenu
import sh.zeron.android.ui.MenuAction

private class Still(val sequence: ULong, val image: ImageBitmap)

private val hideAll = MutableSharedFlow<Unit>(extraBufferCapacity = 1)

private const val MAX_W = 168f
private const val MAX_H = 120f

@Composable
fun ComputerStillPreview(extras: SessionExtras, working: Boolean, onReceiving: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    var still by remember { mutableStateOf<Still?>(null) }
    var dismissed by remember { mutableStateOf(false) }
    var done by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    var enlarged by remember { mutableStateOf(false) }
    val receiving by rememberUpdatedState(onReceiving)
    val isWorking by rememberUpdatedState(working)

    LaunchedEffect(working) {
        if (!working) {
            val finished = still != null && !dismissed
            dismissed = false
            enlarged = false
            receiving(false)
            if (finished) {
                done = true
                delay(1_600)
                done = false
            }
            still = null
            return@LaunchedEffect
        }
        done = false
        while (true) {
            val frame = runCatching { extras.handle.computerStillFrame() }.getOrNull()
            still = when {
                frame == null -> null
                frame.sequence == still?.sequence -> still
                else -> withContext(Dispatchers.Default) {
                    BitmapFactory.decodeByteArray(frame.image, 0, frame.image.size)?.asImageBitmap()
                }?.let { Still(frame.sequence, it) }
            }
            receiving(still != null)
            delay(1_000)
        }
    }
    LaunchedEffect(Unit) { hideAll.collect { if (isWorking) dismissed = true } }

    val shown = still
    val size = shown?.let { fitted(it.image.width.toFloat(), it.image.height.toFloat()) } ?: (MAX_W to MAX_H)
    AnimatedVisibility(
        shown != null && !dismissed,
        enter = fadeIn(tween(180)),
        exit = fadeOut(tween(180)),
        modifier = modifier.offset(x = 11.dp, y = (-11).dp),
    ) {
        Box {
            Box(
                Modifier
                    .padding(top = 11.dp, end = 11.dp)
                    .size(size.first.dp, size.second.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(Color.Black)
                    .border(1.dp, Color.White.copy(alpha = 0.12f), RoundedCornerShape(10.dp))
                    .clickable(onClickLabel = "View larger") { enlarged = true }
                    .semantics { contentDescription = "Computer preview" },
            ) {
                shown?.let { Image(it.image, null, Modifier.fillMaxSize(), contentScale = ContentScale.Fit) }
                if (!done) {
                    Box(
                        Modifier
                            .align(Alignment.BottomStart)
                            .padding(start = 6.dp, bottom = 6.dp)
                            .height(16.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .background(Color.Black.copy(alpha = 0.6f))
                            .padding(horizontal = 6.dp),
                        contentAlignment = Alignment.Center,
                    ) { Text("Live", color = Color.White, fontSize = 10.sp, lineHeight = 10.sp, fontWeight = FontWeight.SemiBold) }
                }
                if (done) {
                    ZIcon(
                        ZIcons.Check,
                        "Computer use finished",
                        Modifier.align(Alignment.Center).size(30.dp).shadow(4.dp, CircleShape),
                        tint = Color.White,
                    )
                }
            }
            Box(
                Modifier
                    .align(Alignment.TopEnd)
                    .size(44.dp)
                    .clickable(onClickLabel = "Hide Computer preview") { menu = true },
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.size(22.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.6f)), contentAlignment = Alignment.Center) {
                    ZIcon(ZIcons.Close, "Hide Computer preview", Modifier.size(12.dp), tint = Color.White)
                }
                ActionMenu(
                    menu,
                    { menu = false },
                    listOf(
                        MenuAction("Hide for this task", ZIcons.Close) { dismissed = true },
                        MenuAction("Hide for all active tasks", ZIcons.Collapse) { hideAll.tryEmit(Unit) },
                    ),
                )
            }
        }
    }
    val big = still?.image
    if (enlarged && big != null) {
        Dialog(onDismissRequest = { enlarged = false }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.85f)).clickable { enlarged = false }, contentAlignment = Alignment.Center) {
                Image(
                    big,
                    "Computer preview",
                    Modifier
                        .padding(16.dp)
                        .fillMaxWidth()
                        .aspectRatio(big.width.toFloat() / big.height.coerceAtLeast(1))
                        .clip(RoundedCornerShape(12.dp)),
                    contentScale = ContentScale.Fit,
                )
            }
        }
    }
}

private fun fitted(w: Float, h: Float): Pair<Float, Float> {
    if (w <= 0f || h <= 0f) return MAX_W to MAX_H
    val scale = minOf(MAX_W / w, MAX_H / h)
    return Math.round(w * scale).toFloat() to Math.round(h * scale).toFloat()
}
