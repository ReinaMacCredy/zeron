package sh.zeron.android.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.zeron_core.FileMatch
import uniffi.zeron_core.InvocationEntry
import uniffi.zeron_core.ThreadMention
import uniffi.zeron_core.nextPastedTextName
import uniffi.zeron_core.pastesAsFile
import uniffi.zeron_core.withQuotes
import uniffi.zeron_core.OutgoingAttachment
import uniffi.zeron_core.fileMentionLink
import java.io.ByteArrayOutputStream
import java.util.UUID
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** An image staged in the composer before sending (JPEG, long side ≤ 2560px). */
class StagedImage(val id: String, val name: String, val bytes: ByteArray, val thumb: ImageBitmap) {
    val outgoing get() = OutgoingAttachment(name, "image/jpeg", bytes)
}

class PastedText(val id: String, val name: String, val text: String) {
    val outgoing get() = OutgoingAttachment(name, "text/plain", text.toByteArray())
}

data class ActiveQuery(val start: Int, val end: Int, val query: String)

@Stable
class ComposerModel(initial: String = "") {
    var value by mutableStateOf(TextFieldValue(initial, TextRange(initial.length)))
    val images = mutableStateListOf<StagedImage>()
    val pasted = mutableStateListOf<PastedText>()
    val quotes = mutableStateListOf<String>()
    private val tokens = LinkedHashMap<String, String>()

    val text: String get() = value.text
    val mentionTokens: Set<String> get() = tokens.keys
    val staged: Int get() = images.size + pasted.size
    val hasContent: Boolean get() = value.text.isNotBlank() || images.isNotEmpty() || pasted.isNotEmpty() || quotes.isNotEmpty()
    val attachments: List<OutgoingAttachment> get() = images.map { it.outgoing } + pasted.map { it.outgoing }

    fun setText(s: String) {
        value = TextFieldValue(s, TextRange(s.length))
    }

    fun clear() {
        setText("")
        images.clear()
        pasted.clear()
        quotes.clear()
        tokens.clear()
    }

    fun stagesPaste(next: TextFieldValue): Boolean {
        val old = value.text
        val added = next.text.length - old.length
        if (added < 2 || staged >= Staging.MAX_IMAGES || !value.selection.collapsed) return false
        val at = value.selection.start
        if (next.text.substring(0, at) != old.substring(0, at) || next.text.substring(at + added) != old.substring(at)) return false
        val inserted = next.text.substring(at, at + added)
        if (!pastesAsFile(inserted, old)) return false
        pasted.add(PastedText(UUID.randomUUID().toString(), nextPastedTextName(images.map { it.name } + pasted.map { it.name }), inserted))
        return true
    }

    fun activeQuery(): Pair<Int, String>? = query('@')?.let { it.start to it.query }

    // A second `/` makes it a path, and `$5` or `$HOME` is not a skill.
    fun activeInvocation(): Pair<ActiveQuery, Boolean>? {
        query('$')?.takeIf { q -> !q.query.contains('/') && q.query.firstOrNull()?.let { it.isDigit() || it.isUpperCase() } != true }?.let { return it to true }
        query('/')?.takeIf { !it.query.contains('/') }?.let { return it to false }
        return null
    }

    private fun query(prefix: Char): ActiveQuery? {
        val t = value.text
        if (!value.selection.collapsed) return null
        val cursor = value.selection.start
        var i = cursor - 1
        while (i >= 0) {
            val c = t[i]
            if (c == prefix) {
                val before = if (i > 0) t[i - 1] else ' '
                return if (before == ' ' || before == '\n') ActiveQuery(i, cursor, t.substring(i + 1, cursor)) else null
            }
            if (c == ' ' || c == '\n') return null
            i--
        }
        return null
    }

    private fun replace(q: ActiveQuery, with: String) {
        val t = value.text
        value = TextFieldValue(t.substring(0, q.start) + with + t.substring(q.end), TextRange(q.start + with.length))
    }

    /** Replace the `@query` with a token for `file` (parent folder on basename clashes). */
    fun insertMention(file: FileMatch) {
        val q = query('@') ?: return
        val link = fileMentionLink(file.path, file.isDir)
        val parts = file.path.trim('/').split('/')
        var token = "@" + (parts.lastOrNull() ?: file.path)
        val existing = tokens[token]
        if (existing != null && existing != link && parts.size > 1) token = "@" + parts.takeLast(2).joinToString("/")
        tokens[token] = link
        replace(q, "$token ")
    }

    fun insertThread(thread: ThreadMention) {
        val q = query('@') ?: return
        val token = "@" + thread.title
        tokens[token] = thread.link
        replace(q, "$token ")
    }

    fun insertInvocation(q: ActiveQuery, entry: InvocationEntry) {
        entry.link?.let { tokens[entry.insert] = it }
        replace(q, entry.insert + " ")
    }

    fun dropQuery(q: ActiveQuery) = replace(q, "")

    fun withoutQuery(q: ActiveQuery): String = value.text.removeRange(q.start, q.end)

    fun beginFileMention() {
        val t = value.text
        val gap = if (t.isEmpty() || t.endsWith(" ") || t.endsWith("\n")) "" else " "
        setText("$t$gap@")
    }

    fun encoded(): String {
        var out = value.text
        for ((token, link) in tokens.entries.sortedByDescending { it.key.length }) {
            if (out.contains(token)) out = out.replace(token, link)
        }
        return withQuotes(out.trim(), quotes.toList())
    }
}

object Staging {
    const val MAX_IMAGES = 8
    private const val MAX_SIDE = 2560
    private const val MAX_BYTES = 24 * 1024 * 1024

    /** Decode, downscale and JPEG-encode a picked image (off the main thread). */
    suspend fun stage(context: Context, uri: Uri): StagedImage? = withContext(Dispatchers.IO) {
        runCatching {
            val source = ImageDecoder.createSource(context.contentResolver, uri)
            val bitmap = ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
                val w = info.size.width
                val h = info.size.height
                val scale = min(1f, MAX_SIDE.toFloat() / max(w, h))
                if (scale < 1f) decoder.setTargetSize((w * scale).roundToInt(), (h * scale).roundToInt())
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }
            stage(bitmap)
        }.getOrNull()
    }

    fun stage(bitmap: Bitmap): StagedImage? {
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 86, out)
        val bytes = out.toByteArray()
        if (bytes.size > MAX_BYTES) return null
        val t = 180f / max(bitmap.width, bitmap.height)
        val thumb = if (t < 1f) Bitmap.createScaledBitmap(bitmap, (bitmap.width * t).roundToInt(), (bitmap.height * t).roundToInt(), true) else bitmap
        val id = UUID.randomUUID().toString()
        return StagedImage(id, "image-${id.take(8)}.jpg", bytes, thumb.asImageBitmap())
    }
}
