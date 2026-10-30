package sh.zeron.android.ui.composer

import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.DrawableRes
import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imeAnimationTarget
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.app.ActivityCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.ComposerModel
import sh.zeron.android.ui.Staging
import sh.zeron.android.ui.composerContainer
import java.io.File

private object KeyboardHeight {
    var px by mutableIntStateOf(0)
}

private object GridPicks {
    val ids = HashMap<Uri, String>()
    var askedForAccess = false

    fun attached(model: ComposerModel): List<Uri> {
        val order = model.images.mapIndexed { i, image -> image.id to i }.toMap()
        return ids.entries.filter { it.value in order }.sortedBy { order[it.value] }.map { it.key }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TrackKeyboardHeight() {
    val target = WindowInsets.imeAnimationTarget.getBottom(LocalDensity.current)
    SideEffect { if (target > 0) KeyboardHeight.px = target }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AttachPanel(model: ComposerModel, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val activity = LocalActivity.current
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val scope = LocalLifecycleOwner.current.lifecycleScope

    BackHandler(onBack = onClose)
    val imeVisible = WindowInsets.isImeVisible
    var sawKeyboardHidden by remember { mutableStateOf(false) }
    val close by rememberUpdatedState(onClose)
    LaunchedEffect(imeVisible) {
        if (!imeVisible) sawKeyboardHidden = true else if (sawKeyboardHidden) close()
    }

    fun stage(uris: List<Uri>, fromGrid: Boolean = false) {
        scope.launch {
            for (uri in uris) {
                if (model.images.size >= Staging.MAX_IMAGES) break
                val image = Staging.stage(context, uri) ?: continue
                model.images.add(image)
                if (fromGrid) GridPicks.ids[uri] = image.id
            }
        }
    }
    fun addThenClose(uris: List<Uri>) {
        stage(uris.take((Staging.MAX_IMAGES - model.images.size).coerceAtLeast(0)))
        onClose()
    }

    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(Staging.MAX_IMAGES)) { addThenClose(it) }
    val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { addThenClose(it) }
    var capture by remember { mutableStateOf<Uri?>(null) }
    val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { saved ->
        addThenClose(listOfNotNull(capture.takeIf { saved }))
    }
    val hasCamera = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
    val clipboard = remember { context.getSystemService(ClipboardManager::class.java) }
    val canPaste = remember { clipboard?.primaryClipDescription?.hasMimeType("image/*") == true }

    var access by remember { mutableStateOf(photoAccess(context)) }
    var recent by remember { mutableStateOf<List<Uri>?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        access = photoAccess(context)
        reload++
    }
    fun requestAccess() {
        val blocked = GridPicks.askedForAccess && access == PhotoAccess.Denied &&
            activity != null && photoPermissions().none { ActivityCompat.shouldShowRequestPermissionRationale(activity, it) }
        GridPicks.askedForAccess = true
        if (blocked) {
            context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)))
        } else {
            permission.launch(photoPermissions())
        }
    }
    LaunchedEffect(Unit) { if (access == PhotoAccess.Denied && !GridPicks.askedForAccess) requestAccess() }
    LifecycleResumeEffect(Unit) {
        val now = photoAccess(context)
        if (now != access) access = now
        reload++
        onPauseOrDispose { }
    }
    LaunchedEffect(access, reload) { recent = if (access == PhotoAccess.Denied) emptyList() else recentPhotos(context) }

    val attached = remember { GridPicks.attached(model) }
    var selected by remember { mutableStateOf(attached) }
    val limit = Staging.MAX_IMAGES - model.images.size + attached.size
    val changed = selected != attached
    val adding = selected.count { it !in attached }

    fun toggle(uri: Uri) {
        selected = when {
            uri in selected -> selected - uri
            selected.size < limit -> selected + uri
            else -> {
                haptics.performHapticFeedback(HapticFeedbackType.Reject)
                return
            }
        }
        haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
    }
    fun commit() {
        val dropped = attached.filter { it !in selected }.mapNotNull { GridPicks.ids[it] }.toSet()
        if (dropped.isNotEmpty()) model.images.removeAll { it.id in dropped }
        stage(selected.filter { it !in attached }, fromGrid = true)
        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
        onClose()
    }

    val full = KeyboardHeight.px.takeIf { it > 0 } ?: with(density) { (300.dp).roundToPx() + WindowInsets.navigationBars.getBottom(this) }
    val covered = maxOf(WindowInsets.ime.getBottom(density), WindowInsets.navigationBars.getBottom(density))
    val navigation = WindowInsets.navigationBars.getBottom(density)
    val shown = with(density) { (full - covered).coerceAtLeast(0).toDp() }
    val content = with(density) { (full - navigation).coerceAtLeast(0).toDp() }

    val background = MaterialTheme.colorScheme.surfaceContainer
    Box(
        modifier
            .fillMaxWidth()
            .height(shown)
            .drawBehind {
                val radius = 28.dp.toPx()
                drawRoundRect(background, size = Size(size.width, size.height + navigation + radius), cornerRadius = CornerRadius(radius))
            }
            .clipToBounds()
            .testTag("attach-panel"),
    ) {
        Column(Modifier.fillMaxWidth().wrapContentHeight(Alignment.Top, unbounded = true).height(content)) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                if (hasCamera) {
                    AttachAction("Camera", ZIcons.Camera) {
                        val dir = File(context.cacheDir, "camera").apply { mkdirs() }
                        val file = File(dir, "capture-${System.currentTimeMillis()}.jpg")
                        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
                        capture = uri
                        camera.launch(uri)
                    }
                }
                AttachAction("Photos", ZIcons.Gallery) { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }
                AttachAction("Files", ZIcons.Folder) { files.launch(arrayOf("image/*")) }
                if (canPaste) {
                    AttachAction("Paste image", ZIcons.Paste) {
                        val uri = clipboard?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri
                            ?.takeIf { context.contentResolver.getType(it)?.startsWith("image/") == true }
                        addThenClose(listOfNotNull(uri))
                    }
                }
            }
            if (access == PhotoAccess.Partial) {
                Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        "Showing the photos you allowed",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = ::requestAccess) { Text("Select more") }
                }
            }
            Box(Modifier.fillMaxWidth().weight(1f)) {
                val list = recent
                when {
                    access == PhotoAccess.Denied -> NoAccess(::requestAccess, Modifier.align(Alignment.Center))
                    list != null && list.isEmpty() -> Text(
                        "No photos yet",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.align(Alignment.Center),
                    )
                    list != null -> RecentPhotosGrid(
                        list,
                        selected,
                        ::toggle,
                        Modifier.fillMaxSize().testTag("attach-grid"),
                        contentPadding = PaddingValues(bottom = 76.dp),
                    )
                }
                AnimatedContent(
                    when {
                        !changed -> "All Photos"
                        adding == 0 -> "Done"
                        else -> "Add $adding"
                    },
                    modifier = Modifier.align(Alignment.BottomEnd).padding(12.dp),
                    label = "attach-main",
                ) { title ->
                    if (title == "All Photos") {
                        FilledTonalButton(
                            onClick = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                            modifier = Modifier.height(48.dp).testTag("attach-grid-main"),
                        ) { Text(title) }
                    } else {
                        Button(onClick = ::commit, modifier = Modifier.height(48.dp).testTag("attach-grid-main")) { Text(title) }
                    }
                }
            }
        }
    }
}

@Composable
private fun AttachAction(label: String, @DrawableRes icon: Int, onClick: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.width(76.dp)) {
        Surface(
            onClick = onClick,
            shape = RoundedCornerShape(20.dp),
            color = composerContainer(),
            contentColor = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.size(56.dp),
        ) {
            Box(contentAlignment = Alignment.Center) { ZIcon(icon, label, Modifier.size(24.dp)) }
        }
        Spacer(Modifier.height(6.dp))
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            maxLines = 1,
        )
    }
}

@Composable
private fun NoAccess(onAllow: () -> Unit, modifier: Modifier) {
    Column(modifier.padding(horizontal = 32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            "Allow access to show your recent photos here.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(12.dp))
        FilledTonalButton(onClick = onAllow) { Text("Allow access") }
    }
}
