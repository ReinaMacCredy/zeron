package sh.zeron.android.core

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONObject
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.DeviceView
import uniffi.zeron_core.WallpaperEffect
import uniffi.zeron_core.wallpaperRender
import uniffi.zeron_core.wallpaperSafeOpacity
import java.io.File
import java.nio.ByteBuffer
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * The chat wallpaper (desktop "new thread background"): one image plus an
 * effect, stored locally. Effects and the contrast guard run in the Rust
 * core (`wallpaper.rs`) off the main thread; renders are cached per
 * appearance — the same pipeline as iOS.
 */
class WallpaperStore(private val context: Context) {
    data class State(
        val set: Boolean,
        val name: String?,
        val effect: WallpaperEffect,
        val generation: Int,
        val chat: ChatBackground = ChatBackground(),
        val syncWithMac: Boolean = false,
        val following: Synced? = null,
    )

    enum class Placement(val key: String, val label: String, val detail: String) {
        HERO("hero", "Hero", "Same top crop as new threads, fading out below."),
        FULL("full", "Full", "Covers the whole conversation column."),
    }

    /** The desktop's chat background: the artwork behind open conversations. */
    data class ChatBackground(
        val enabled: Boolean = false,
        val placement: Placement = Placement.HERO,
        val opacity: Float = 0.2f,
        val blur: Float = 24f,
        val scrim: Boolean = true,
    ) {
        val visible: Boolean get() = enabled && opacity > 0f

        fun clamped(): ChatBackground {
            val b = if (blur.isFinite()) blur.coerceIn(0f, BLUR_MAX) else 24f
            return copy(
                opacity = if (opacity.isFinite()) opacity.coerceIn(0f, 1f) else 0.2f,
                blur = (b / BLUR_STEP).roundToInt() * BLUR_STEP,
            )
        }

        companion object {
            const val BLUR_MAX = 48f
            const val BLUR_STEP = 4f
        }
    }

    /** A Mac's artwork and chat settings, followed while Sync with Mac is on. */
    data class Synced(val deviceId: String, val name: String?, val hash: String?, val effect: String, val chat: ChatBackground)

    /** A rendered wallpaper and the highest opacity that keeps text readable. */
    class Render(val image: ImageBitmap, val opacity: Float)

    private val prefs = context.getSharedPreferences("wallpaper", 0)
    private val dir = File(context.filesDir, "wallpaper").apply { mkdirs() }
    private val file = File(dir, "wallpaper.jpg")
    private val syncedFile = File(dir, "synced.jpg")
    private val cache = HashMap<String, Render>()
    private var inFlight = false

    private val _state = MutableStateFlow(read(0))
    val state: StateFlow<State> = _state.asStateFlow()

    private fun read(generation: Int): State {
        val sync = prefs.getBoolean("sync", false)
        val following = if (sync) readSynced() else null
        val local = ChatBackground(
            enabled = prefs.getBoolean("chat", false),
            placement = Placement.entries.firstOrNull { it.key == prefs.getString("chatPlacement", null) } ?: Placement.HERO,
            opacity = prefs.getFloat("chatOpacity", 0.2f),
            blur = prefs.getFloat("chatBlur", 24f),
            scrim = prefs.getBoolean("chatScrim", true),
        ).clamped()
        return State(
            set = (if (following != null) syncedFile else file).exists(),
            name = if (following != null) following.name else prefs.getString("name", null),
            effect = effectNamed(following?.effect ?: prefs.getString("effect", "NONE")),
            generation = generation,
            chat = following?.chat ?: local,
            syncWithMac = sync,
            following = following,
        )
    }

    private fun readSynced(): Synced? = runCatching {
        val o = JSONObject(prefs.getString("synced", null) ?: return null)
        Synced(
            deviceId = o.getString("deviceId"),
            name = o.optString("name").ifEmpty { null },
            hash = o.optString("hash").ifEmpty { null },
            effect = o.optString("effect", "none"),
            chat = ChatBackground(
                enabled = o.optBoolean("enabled"),
                placement = Placement.entries.firstOrNull { it.key == o.optString("placement") } ?: Placement.HERO,
                opacity = o.optDouble("opacity", 0.2).toFloat(),
                blur = o.optDouble("blur", 24.0).toFloat(),
                scrim = o.optBoolean("scrim", true),
            ).clamped(),
        )
    }.getOrNull()

    private fun writeSynced(s: Synced) {
        val o = JSONObject()
            .put("deviceId", s.deviceId)
            .put("name", s.name ?: "")
            .put("hash", s.hash ?: "")
            .put("effect", s.effect)
            .put("enabled", s.chat.enabled)
            .put("placement", s.chat.placement.key)
            .put("opacity", s.chat.opacity.toDouble())
            .put("blur", s.chat.blur.toDouble())
            .put("scrim", s.chat.scrim)
        prefs.edit().putString("synced", o.toString()).apply()
    }

    fun setChat(chat: ChatBackground) {
        val c = chat.clamped()
        prefs.edit()
            .putBoolean("chat", c.enabled)
            .putString("chatPlacement", c.placement.key)
            .putFloat("chatOpacity", c.opacity)
            .putFloat("chatBlur", c.blur)
            .putBoolean("chatScrim", c.scrim)
            .apply()
        _state.value = read(_state.value.generation)
    }

    fun setSyncWithMac(on: Boolean) {
        prefs.edit().putBoolean("sync", on).apply()
        changed()
    }

    val syncDeviceId: String? get() = prefs.getString("syncDevice", null)

    fun setSyncDevice(id: String) {
        prefs.edit().putString("syncDevice", id).apply()
        changed()
    }

    /** The Mac to follow: the one picked, else the most recently online. */
    fun host(client: CoreClient): DeviceView? {
        val hosts = client.wallpaperHosts()
        return hosts.firstOrNull { it.id == syncDeviceId } ?: hosts.firstOrNull()
    }

    /** Fetch the followed Mac's wallpaper; the image only when its hash changed. */
    suspend fun refresh(client: CoreClient) {
        if (!_state.value.syncWithMac || inFlight) return
        val host = host(client) ?: return
        inFlight = true
        try {
            val previous = readSynced()
            val have = previous?.deviceId == host.id && syncedFile.exists()
            val fetched = runCatching { client.chatWallpaper(host.id, if (have) previous?.hash else null) }.getOrNull() ?: return
            val chat = ChatBackground(
                enabled = fetched.enabled,
                placement = Placement.entries.firstOrNull { it.key == fetched.placement } ?: Placement.HERO,
                opacity = fetched.opacity,
                blur = fetched.blur,
                scrim = fetched.scrim,
            ).clamped()
            val next = Synced(host.id, fetched.name, fetched.imageHash, fetched.effect, chat)
            val image = fetched.image
            withContext(Dispatchers.IO) {
                if (image != null) syncedFile.writeBytes(image) else if (next.hash == null) syncedFile.delete()
            }
            if (image == null && next == previous) return
            writeSynced(next)
            changed()
        } finally {
            inFlight = false
        }
    }

    private fun changed() {
        synchronized(cache) { cache.clear() }
        _state.value = read(_state.value.generation + 1)
    }

    /** Store a picked image (long side ≤ 1600 px, decoded downsampled). */
    suspend fun set(uri: Uri, name: String) = withContext(Dispatchers.IO) {
        runCatching {
            val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                val scale = min(1f, 1600f / max(info.size.width, info.size.height))
                if (scale < 1f) decoder.setTargetSize((info.size.width * scale).roundToInt(), (info.size.height * scale).roundToInt())
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }
            store(bitmap, name)
        }
    }

    /** Store an image file (launch extra `wallpaper` for screenshots and tests). */
    suspend fun set(path: File) = withContext(Dispatchers.IO) {
        runCatching {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(path.path, bounds)
            var sample = 1
            while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= 1600) sample *= 2
            val bitmap = BitmapFactory.decodeFile(path.path, BitmapFactory.Options().apply { inSampleSize = sample })!!
            store(bitmap, path.name)
        }
    }

    private suspend fun store(bitmap: Bitmap, name: String) {
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, 90, it) }
        prefs.edit().putString("name", name).apply()
        withContext(Dispatchers.Main) { changed() }
    }

    fun setEffect(effect: WallpaperEffect) {
        prefs.edit().putString("effect", effect.name).apply()
        changed()
    }

    fun remove() {
        file.delete()
        prefs.edit().remove("name").apply()
        changed()
    }

    /** Render for an appearance (null when no wallpaper is set). */
    suspend fun render(dark: Boolean): Render? {
        val s = _state.value
        if (!s.set) return null
        val key = "${s.effect}-$dark"
        synchronized(cache) { cache[key] }?.let { return it }
        return withContext(Dispatchers.Default) {
            runCatching {
                val source = BitmapFactory.decodeFile((if (s.following != null) syncedFile else file).path) ?: return@runCatching null
                val scale = min(1f, 1200f / max(source.width, source.height))
                val w = max(1, (source.width * scale).roundToInt())
                val h = max(1, (source.height * scale).roundToInt())
                val sized = Bitmap.createScaledBitmap(source, w, h, true).copy(Bitmap.Config.ARGB_8888, false)
                val buffer = ByteBuffer.allocate(w * h * 4)
                sized.copyPixelsToBuffer(buffer)
                val out = wallpaperRender(buffer.array(), w.toUInt(), h.toUInt(), s.effect, !dark)
                // Text sits over the top of the artwork: primary text keeps
                // 4.5:1, secondary 3:1 against its worst pixels there.
                val text = (if (dark) 0xE8E8EA else 0x27272C).toUInt()
                val secondary = (if (dark) 0xA9A9AE else 0x62626A).toUInt()
                val background = (if (dark) 0x060606 else 0xF3F3F5).toUInt()
                val primary = wallpaperSafeOpacity(out, w.toUInt(), h.toUInt(), text, background, 0.6f, 4.5f, 1f)
                val muted = wallpaperSafeOpacity(out, w.toUInt(), h.toUInt(), secondary, background, 0.6f, 3f, 1f)
                val rendered = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
                rendered.copyPixelsFromBuffer(ByteBuffer.wrap(out))
                Render(rendered.asImageBitmap(), min(primary, muted))
            }.getOrNull()
        }?.also { r -> if (_state.value.generation == s.generation) synchronized(cache) { cache[key] = r } }
    }

    companion object {
        fun effectNamed(name: String?): WallpaperEffect =
            runCatching { WallpaperEffect.valueOf(name!!.uppercase()) }.getOrDefault(WallpaperEffect.NONE)

        val effects = listOf(WallpaperEffect.NONE, WallpaperEffect.DITHER, WallpaperEffect.ASCII, WallpaperEffect.HALFTONE, WallpaperEffect.SCANLINES)

        fun label(e: WallpaperEffect) = when (e) {
            WallpaperEffect.NONE -> "None"
            WallpaperEffect.DITHER -> "Dither"
            WallpaperEffect.ASCII -> "ASCII"
            WallpaperEffect.HALFTONE -> "Halftone"
            WallpaperEffect.SCANLINES -> "Scanlines"
        }

        fun detail(e: WallpaperEffect) = when (e) {
            WallpaperEffect.NONE -> "Shows the original artwork."
            WallpaperEffect.DITHER -> "Rebuilds the artwork with a dithered color palette."
            WallpaperEffect.ASCII -> "Recreates the artwork with colored characters."
            WallpaperEffect.HALFTONE -> "Recreates the artwork with colored print dots."
            WallpaperEffect.SCANLINES -> "Adds a pronounced horizontal display-line texture."
        }
    }
}
