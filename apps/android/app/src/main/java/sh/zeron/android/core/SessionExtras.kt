package sh.zeron.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.ChipWrite
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.ModelSwitchPrompt
import uniffi.zeron_core.SandboxLevel
import uniffi.zeron_core.SessionHandle

class SessionExtras(val model: AppModel, val core: CoreClient, val handle: SessionHandle, val chatId: String) {
    class PendingSwitch(val prompt: ModelSwitchPrompt, val apply: suspend () -> Unit)

    private val scope = MainScope()

    var pendingSwitch by mutableStateOf<PendingSwitch?>(null)

    var compactNotice by mutableStateOf<String?>(null)

    val runsClaudeCode: Boolean get() = (model.row(chatId)?.harness ?: "claude-code") == "claude-code"

    val hostDevice: String get() = runCatching { handle.composer().host.deviceId }.getOrDefault("")

    fun config(): ChatConfig {
        val row = model.row(chatId)
        return core.sessionConfig(chatId) ?: ChatConfig(row?.harness ?: "claude-code", row?.model, row?.reasoning, emptyMap(), SandboxLevel.WORKSPACE_WRITE)
    }

    fun setConfig(change: (ChatConfig) -> ChatConfig) {
        runCatching { core.setSessionConfig(chatId, change(config())) }.onFailure { model.toast(failure(it, "Couldn't change the session. Try again.")) }
    }

    fun confirmSwitch(prompt: ModelSwitchPrompt?, change: (ChatConfig) -> ChatConfig) {
        if (prompt == null) return setConfig(change)
        pendingSwitch = PendingSwitch(prompt) { core.setSessionConfigNoted(chatId, change(config()), prompt.note, prompt.noteHint) }
    }

    fun resolveSwitch(confirm: Boolean) {
        val pending = pendingSwitch ?: return
        pendingSwitch = null
        if (confirm) launch("Couldn't switch. Try again.") { pending.apply() }
    }

    fun applyChip(write: ChipWrite) {
        write.model?.let { id ->
            val row = model.row(chatId)
            val from = row?.modelLabel ?: core.sessionConfig(chatId)?.model ?: ""
            confirmSwitch(core.modelSwitchPrompt(chatId, id, from, write.modelLabel ?: id)) { it.copy(model = id) }
        }
        write.reasoning?.let { r -> setConfig { it.copy(reasoning = r) } }
        write.permission?.let { p -> setConfig { it.copy(access = p) } }
    }

    fun compact(focus: String? = null) {
        compactNotice = null
        scope.launch {
            compactNotice = try {
                handle.compact(focus)
            } catch (e: Exception) {
                "Compaction failed: ${failure(e, e.message ?: "$e")}"
            }
        }
    }

    fun launch(fallback: String, work: suspend () -> Unit) {
        scope.launch {
            try {
                work()
            } catch (e: Exception) {
                model.toast(failure(e, fallback))
            }
        }
    }

    fun close() {
        scope.coroutineContext[kotlinx.coroutines.Job]?.cancel()
    }

    companion object {
        fun failure(e: Throwable, fallback: String): String = when (e) {
            is CoreException.NotFound -> e.reason
            is CoreException.HostException -> e.reason
            else -> fallback
        }
    }
}
