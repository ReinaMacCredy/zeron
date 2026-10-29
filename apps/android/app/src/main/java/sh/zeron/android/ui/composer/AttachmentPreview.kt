package sh.zeron.android.ui.composer

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.DialogSystemBars
import sh.zeron.android.ui.StagedImage
import sh.zeron.android.ui.TonalCircleButton

@Composable
fun AttachmentPreview(image: StagedImage, index: Int, count: Int, onRemove: () -> Unit, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        DialogSystemBars(lightIcons = true)
        val bitmap by produceState<ImageBitmap>(image.thumb, image.id) {
            withContext(Dispatchers.Default) { runCatching { BitmapFactory.decodeByteArray(image.bytes, 0, image.bytes.size)?.asImageBitmap() }.getOrNull() }
                ?.let { value = it }
        }
        val label = "Image ${index + 1} of $count"
        Box(Modifier.fillMaxSize().background(Color.Black).testTag("attachment-preview")) {
            Image(
                bitmap,
                contentDescription = label,
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(top = 72.dp, bottom = 132.dp),
            )
            Box(Modifier.fillMaxSize().safeDrawingPadding().padding(12.dp)) {
                TonalCircleButton(
                    ZIcons.Close, "Close preview", onClick = onDismiss,
                    modifier = Modifier.align(Alignment.TopStart).testTag("attachment-preview-close"), size = 48.dp,
                    container = Color.White.copy(alpha = 0.16f), content = Color.White,
                )
                Text(
                    "${index + 1} / $count",
                    style = MaterialTheme.typography.titleSmall,
                    color = Color.White.copy(alpha = 0.7f),
                    modifier = Modifier.align(Alignment.TopCenter).padding(top = 14.dp).semantics { contentDescription = label },
                )
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 32.dp),
                ) {
                    Surface(
                        onClick = {
                            onRemove()
                            onDismiss()
                        },
                        shape = CircleShape,
                        color = Color(0xCC3C3C40),
                        contentColor = Color.White,
                        modifier = Modifier.size(56.dp).testTag("attachment-preview-remove"),
                    ) {
                        Box(contentAlignment = Alignment.Center) { ZIcon(ZIcons.Delete, "Remove image", Modifier.size(24.dp)) }
                    }
                    Spacer(Modifier.height(8.dp))
                    Text("Remove", style = MaterialTheme.typography.labelLarge, color = Color.White)
                }
            }
        }
    }
}
