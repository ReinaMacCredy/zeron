package sh.zeron.android.ui.composer

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.util.LruCache
import android.util.Size
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal enum class PhotoAccess { Full, Partial, Denied }

internal fun photoPermissions(): Array<String> = when {
    Build.VERSION.SDK_INT >= 34 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
    Build.VERSION.SDK_INT >= 33 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES)
    else -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
}

internal fun photoAccess(context: Context): PhotoAccess {
    fun granted(p: String) = ContextCompat.checkSelfPermission(context, p) == PackageManager.PERMISSION_GRANTED
    return when {
        Build.VERSION.SDK_INT >= 33 && granted(Manifest.permission.READ_MEDIA_IMAGES) -> PhotoAccess.Full
        Build.VERSION.SDK_INT >= 34 && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) -> PhotoAccess.Partial
        Build.VERSION.SDK_INT < 33 && granted(Manifest.permission.READ_EXTERNAL_STORAGE) -> PhotoAccess.Full
        else -> PhotoAccess.Denied
    }
}

internal suspend fun recentPhotos(context: Context, limit: Int = 240): List<Uri> = withContext(Dispatchers.IO) {
    val base = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    runCatching {
        context.contentResolver.query(base, arrayOf(MediaStore.Images.Media._ID), null, null, "${MediaStore.Images.Media.DATE_ADDED} DESC")?.use { cursor ->
            val id = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            buildList {
                while (size < limit && cursor.moveToNext()) add(ContentUris.withAppendedId(base, cursor.getLong(id)))
            }
        }
    }.getOrNull() ?: emptyList()
}

private object Thumbnails {
    val cache = LruCache<Uri, ImageBitmap>(180)

    suspend fun load(context: Context, uri: Uri): ImageBitmap? = cache.get(uri) ?: withContext(Dispatchers.IO) {
        runCatching { context.contentResolver.loadThumbnail(uri, Size(320, 320), null).asImageBitmap() }
            .getOrNull()
            ?.also { cache.put(uri, it) }
    }
}

@Composable
internal fun RecentPhotosGrid(
    photos: List<Uri>,
    selected: List<Uri>,
    onToggle: (Uri) -> Unit,
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    LazyVerticalGrid(
        GridCells.Fixed(3),
        modifier = modifier.semantics { contentDescription = "Recent photos" },
        contentPadding = contentPadding,
        horizontalArrangement = Arrangement.spacedBy(2.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        items(photos, key = { it.toString() }) { uri ->
            val order = selected.indexOf(uri).takeIf { it >= 0 }?.plus(1)
            PhotoCell(uri, order) { onToggle(uri) }
        }
    }
}

@Composable
private fun PhotoCell(uri: Uri, order: Int?, onClick: () -> Unit) {
    val context = LocalContext.current
    var thumb by remember(uri) { mutableStateOf(Thumbnails.cache.get(uri)) }
    LaunchedEffect(uri) { if (thumb == null) thumb = Thumbnails.load(context, uri) }
    val scale by animateFloatAsState(if (order != null) 0.9f else 1f, label = "photo-scale")
    Box(
        Modifier
            .aspectRatio(1f)
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .semantics {
                contentDescription = if (order != null) "Photo, selected $order" else "Photo"
                selected = order != null
            }
            .clickable(onClick = onClick),
    ) {
        thumb?.let {
            Image(
                it,
                null,
                contentScale = ContentScale.Crop,
                modifier = Modifier
                    .fillMaxSize()
                    .graphicsLayer {
                        scaleX = scale
                        scaleY = scale
                    }
                    .clip(MaterialTheme.shapes.small),
            )
        }
        Badge(order, Modifier.align(Alignment.TopEnd).padding(6.dp))
    }
}

@Composable
private fun Badge(order: Int?, modifier: Modifier) {
    if (order == null) {
        Box(
            modifier
                .size(24.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.18f))
                .border(1.5.dp, Color.White, CircleShape),
        )
    } else {
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.primary,
            contentColor = MaterialTheme.colorScheme.onPrimary,
            border = BorderStroke(1.5.dp, Color.White),
            modifier = modifier.size(24.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Text("$order", fontSize = 13.sp, fontWeight = FontWeight.Bold)
            }
        }
    }
}
