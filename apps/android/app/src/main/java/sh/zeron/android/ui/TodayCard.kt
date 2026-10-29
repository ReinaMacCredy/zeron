package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.HomeModel
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.TodayPage
import uniffi.zeron_core.TodayPart
import uniffi.zeron_core.TodayUsage

private fun TodayUsage.lead() = "Left on $harnessName, ${window.lowercase()}"

private fun TodayUsage.remaining() = Math.round((1 - usedFraction) * 100)

@Composable
private fun TodayUsage.tone(): Color = when {
    usedFraction >= 0.95f -> MaterialTheme.colorScheme.error
    usedFraction >= 0.8f -> if (LocalDarkTheme.current) Color(0xFFFBBF24) else Color(0xFFB45309)
    else -> MaterialTheme.colorScheme.primary
}

@Composable
private fun headline(page: TodayPage): AnnotatedString {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    return buildAnnotatedString {
        pushStyle(SpanStyle(fontWeight = FontWeight.SemiBold))
        append(page.greeting + ". ")
        pop()
        pushStyle(SpanStyle(color = muted))
        append(page.summary)
        pop()
    }
}

@Composable
private fun digest(page: TodayPage): AnnotatedString {
    val bad = MaterialTheme.colorScheme.error
    return buildAnnotatedString {
        page.digest.forEachIndexed { i, sentence ->
            if (i > 0) append(" ")
            for (part in sentence.parts) when (part) {
                is TodayPart.Text -> append(part.text)
                is TodayPart.Strong -> { pushStyle(SpanStyle(fontWeight = FontWeight.SemiBold)); append(part.text); pop() }
                is TodayPart.Bad -> { pushStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = bad)); append(part.text); pop() }
            }
        }
    }
}

@Composable
fun TodayCard(page: TodayPage, onOpen: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        onClick = onOpen,
        shape = RoundedCornerShape(24.dp),
        color = cardColor(),
        modifier = modifier
            .fillMaxWidth()
            .semantics { contentDescription = "Today. ${page.greeting}. ${page.summary}" },
    ) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp, bottom = 14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(headline(page), style = MaterialTheme.typography.bodyLarge)
            Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Tile("Today's tasks", "${page.openTasks}", "open")
                Tile("Your day so far", "${page.prompts}", if (page.prompts == 1u) "prompt" else "prompts")
                page.usage?.let { Tile(it.lead(), "${it.remaining()}%", null) }
            }
        }
    }
}

@Composable
private fun androidx.compose.foundation.layout.RowScope.Tile(key: String, value: String, unit: String?) {
    Column(
        Modifier
            .weight(1f)
            .fillMaxHeight()
            .heightIn(min = 62.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(key, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
        Spacer(Modifier.height(2.dp))
        Text(
            buildAnnotatedString {
                pushStyle(SpanStyle(fontWeight = FontWeight.Bold))
                append(value)
                pop()
                if (unit != null) {
                    pushStyle(SpanStyle(fontSize = MaterialTheme.typography.labelSmall.fontSize, color = MaterialTheme.colorScheme.onSurfaceVariant))
                    append(" $unit")
                    pop()
                }
            },
            style = MaterialTheme.typography.titleMedium,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun TodaySheet(page: TodayPage, home: HomeModel, onAllTasks: () -> Unit, onDismiss: () -> Unit) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val pending = remember { mutableStateMapOf<String, Boolean>() }
    var draft by remember { mutableStateOf("") }
    LaunchedEffect(page) { page.tasks.forEach { t -> if (pending[t.id] == t.done) pending.remove(t.id) } }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Text("Today", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 24.dp, bottom = 8.dp))
        LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 32.dp), modifier = Modifier.imePadding()) {
            item("tasks-h") {
                SheetHeader("Today's tasks · ${page.openTasks}") {
                    TextButton(onClick = { onDismiss(); onAllTasks() }) { Text("All tasks") }
                }
            }
            val rows = page.tasks.size.coerceAtLeast(1) + 1
            if (page.tasks.isEmpty()) {
                item("none") {
                    SheetRow(0, rows) { Text("Nothing due today", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
            items(page.tasks.size, key = { "t-" + page.tasks[it].id }) { i ->
                val task = page.tasks[i]
                val done = pending[task.id] ?: task.done
                SheetRow(i, rows, onClick = {
                    pending[task.id] = !done
                    scope.launch {
                        runCatching { home.setTaskCompleted(task.id, !done) }.onFailure { pending.remove(task.id) }
                    }
                }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (done) {
                            Box(Modifier.size(22.dp).clip(CircleShape).background(successColor()), contentAlignment = Alignment.Center) {
                                ZIcon(ZIcons.Check, "Done", Modifier.size(15.dp), tint = MaterialTheme.colorScheme.surface)
                            }
                        } else {
                            Box(Modifier.size(22.dp).border(2.dp, MaterialTheme.colorScheme.outline, CircleShape))
                        }
                        Spacer(Modifier.width(14.dp))
                        Column {
                            Text(
                                task.title,
                                style = MaterialTheme.typography.bodyLarge,
                                color = if (done) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.onSurface,
                                textDecoration = if (done) TextDecoration.LineThrough else null,
                            )
                            val host = task.host?.name
                            val meta = listOfNotNull(task.meta, host).joinToString(" · ")
                            if (meta.isNotEmpty()) {
                                Text(
                                    meta,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = if (task.overdue && !done) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
            item("add") {
                SheetRow(rows - 1, rows) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ZIcon(ZIcons.Plus, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.outline)
                        Spacer(Modifier.width(14.dp))
                        Box(Modifier.weight(1f)) {
                            if (draft.isEmpty()) Text("Add a task for today", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.outline)
                            BasicTextField(
                                draft,
                                { draft = it },
                                singleLine = true,
                                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                                keyboardActions = KeyboardActions(onDone = {
                                    val title = draft.trim()
                                    if (title.isEmpty()) return@KeyboardActions
                                    draft = ""
                                    scope.launch {
                                        runCatching { home.addTodayTask(title) }.onFailure { if (draft.isEmpty()) draft = title }
                                    }
                                }),
                                modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Add a task for today" },
                            )
                        }
                    }
                }
            }
            item("day-h") {
                SheetHeader("Your day so far") {
                    page.updated?.let { Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline) }
                }
            }
            item("day") {
                SheetRow(0, 1) {
                    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (page.runsLoaded || page.digest.isNotEmpty()) {
                            Text(digest(page), style = MaterialTheme.typography.bodyMedium)
                        } else {
                            Text("Loading your day…", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        val peak = maxOf(1u, page.slots.maxOfOrNull { it.prompts } ?: 0u).toFloat()
                        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            for (slot in page.slots) {
                                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text(
                                        if (slot.now) "${slot.label}, now" else slot.label,
                                        style = MaterialTheme.typography.labelSmall,
                                        fontWeight = if (slot.now) FontWeight.SemiBold else null,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                    )
                                    Bar(slot.prompts.toFloat() / peak, MaterialTheme.colorScheme.primary)
                                    Text(
                                        if (slot.prompts == 0u) " " else "${slot.prompts} ${if (slot.prompts == 1u) "prompt" else "prompts"}",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 1,
                                    )
                                }
                            }
                        }
                    }
                }
            }
            page.usage?.let { usage ->
                item("usage-h") { SheetHeader("Left on usage") {} }
                item("usage") {
                    SheetRow(0, 1) {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                HarnessMark(usage.harness, 18.dp, tint = MaterialTheme.colorScheme.onSurface)
                                Spacer(Modifier.width(8.dp))
                                Text(usage.lead(), style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                                Text("${usage.remaining()}%", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                            }
                            Bar(1 - usage.usedFraction, usage.tone())
                            Text(
                                listOf(usage.resetsIn?.let { "Resets in $it" } ?: "Your tightest limit", usage.host.name).joinToString(" · "),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun SheetHeader(title: String, trailing: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
        trailing()
    }
}

@Composable
private fun SheetRow(index: Int, count: Int, onClick: (() -> Unit)? = null, content: @Composable () -> Unit) {
    val big = 20.dp
    val small = 4.dp
    val shape = RoundedCornerShape(
        topStart = if (index == 0) big else small,
        topEnd = if (index == 0) big else small,
        bottomStart = if (index == count - 1) big else small,
        bottomEnd = if (index == count - 1) big else small,
    )
    Box(
        Modifier
            .padding(bottom = 2.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(cardColor())
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 14.dp),
    ) { content() }
}

@Composable
private fun Bar(fraction: Float, tone: Color) {
    Box(Modifier.fillMaxWidth().height(5.dp).clip(RoundedCornerShape(3.dp)).background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
        Box(Modifier.fillMaxWidth(fraction.coerceIn(0.001f, 1f)).height(5.dp).clip(RoundedCornerShape(3.dp)).background(tone))
    }
}
