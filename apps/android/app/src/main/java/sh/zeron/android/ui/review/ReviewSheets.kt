package sh.zeron.android.ui.review

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.SessionExtras
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.SectionHeader
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.git.DiffDelta
import sh.zeron.android.ui.git.DiffNotice
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.ReviewerOptionRow
import uniffi.zeron_core.ReviewerPick

@Composable
internal fun rememberReviewRevision(extras: SessionExtras): Int {
    var revision by remember { mutableIntStateOf(0) }
    LaunchedEffect(extras.chatId) { extras.model.sessionEvents.collect { if (it == extras.chatId) revision++ } }
    return revision
}

private sealed interface OptionsPage {
    data object Options : OptionsPage
    data object Commits : OptionsPage
    data class Models(val harness: String) : OptionsPage
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReviewOptionsSheet(extras: SessionExtras, onDismiss: () -> Unit) {
    val handle = extras.handle
    var page by remember { mutableStateOf<OptionsPage>(OptionsPage.Options) }
    var local by remember { mutableIntStateOf(0) }
    val revision = rememberReviewRevision(extras)
    LaunchedEffect(Unit) {
        runCatching { handle.openReviewOptions() }.onFailure { extras.model.toast(SessionExtras.failure(it, "Couldn't load the review options. Try again.")) }
        local++
    }
    val options = remember(revision, local) { handle.reviewOptions() }
    fun write(fallback: String, change: () -> Unit) {
        runCatching(change).onFailure { extras.model.toast(SessionExtras.failure(it, fallback)) }
        local++
    }
    BackHandler(page != OptionsPage.Options) { page = OptionsPage.Options }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        when (val p = page) {
            OptionsPage.Options -> LazyColumn(Modifier.fillMaxWidth().testTag("review-options")) {
                item { SheetTitle(options?.title ?: "Review changes") }
                item {
                    Button(
                        onClick = {
                            onDismiss()
                            extras.launch("Couldn't start the review. Try again.") { handle.startReview() }
                        },
                        enabled = options?.busy == false,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).height(50.dp).testTag("review-start"),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.onSurface,
                            contentColor = MaterialTheme.colorScheme.surface,
                        ),
                    ) { Text(options?.startLabel ?: "Review") }
                }
                if (options != null) {
                    item { SectionHeader(options.scopeHeading) }
                    val count = options.scopes.size + 1
                    itemsIndexed(options.scopes, key = { _, s -> "scope:${s.key}" }) { i, s ->
                        OptionRow(
                            index = i,
                            count = count,
                            title = s.label,
                            supporting = s.note,
                            enabled = s.enabled,
                            leading = { Check(s.selected) },
                            trailing = if (s.additions != null && s.deletions != null) {
                                { DiffDelta(s.additions!!, s.deletions!!) }
                            } else {
                                null
                            },
                            tag = "review-scope-${s.key}",
                        ) { write("Couldn't change the scope. Try again.") { handle.setReviewScope(s.key) } }
                    }
                    item(key = "scope:commit") {
                        OptionRow(
                            index = count - 1,
                            count = count,
                            title = options.commitLabel,
                            supporting = options.commitPicked,
                            leading = { Check(options.commitSelected) },
                            trailing = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline) },
                            tag = "review-scope-commit",
                        ) { page = OptionsPage.Commits }
                    }
                    item { SectionHeader(options.reviewerHeading) }
                    itemsIndexed(options.reviewers) { i, r ->
                        OptionRow(
                            index = i,
                            count = options.reviewers.size,
                            title = r.label,
                            supporting = r.detail,
                            leading = {
                                val brand = r.brand
                                if (brand != null) {
                                    HarnessMark(brand, 18.dp)
                                } else {
                                    ZIcon(if (r.pick is ReviewerPick.Auto) ZIcons.Magic else ZIcons.Bot, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            },
                            trailing = if (r.selected || r.modelsHarness != null) {
                                {
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        if (r.selected) ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary)
                                        r.modelsHarness?.let { harness ->
                                            IconButton(onClick = { page = OptionsPage.Models(harness) }, modifier = Modifier.testTag("review-reviewer-models-$i")) {
                                                ZIcon(ZIcons.ChevronRight, "Models", Modifier.size(18.dp), tint = MaterialTheme.colorScheme.outline)
                                            }
                                        }
                                    }
                                }
                            } else {
                                null
                            },
                            tag = "review-reviewer-$i",
                        ) { write("Couldn't change the reviewer. Try again.") { handle.setReviewer(r.pick) } }
                    }
                }
                item { Box(Modifier.navigationBarsPadding().height(16.dp)) }
            }
            OptionsPage.Commits -> CommitsPage(extras, options?.commitLabel ?: "Commit", revision) {
                local++
                page = OptionsPage.Options
            }
            is OptionsPage.Models -> ModelsPage(extras, p.harness, revision) { page = OptionsPage.Options }
        }
    }
}

@Composable
private fun CommitsPage(extras: SessionExtras, title: String, revision: Int, onBack: () -> Unit) {
    val handle = extras.handle
    var query by remember { mutableStateOf("") }
    LaunchedEffect(Unit) {
        runCatching { handle.loadReviewCommits() }.onFailure { extras.model.toast(SessionExtras.failure(it, "Couldn't load the commits. Try again.")) }
    }
    val commits = remember(revision, query) { handle.reviewCommits(query) }
    LazyColumn(Modifier.fillMaxWidth().testTag("review-commits")) {
        item { SheetTitle(title, onBack) }
        item {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search") },
                leadingIcon = { ZIcon(ZIcons.Search, null, Modifier.size(18.dp)) },
                singleLine = true,
                shape = RoundedCornerShape(24.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
        when {
            commits.loading && commits.rows.isEmpty() -> item { Spinner() }
            commits.rows.isEmpty() -> item { DiffNotice(commits.empty) }
        }
        itemsIndexed(commits.rows, key = { _, c -> c.sha }) { i, c ->
            OptionRow(
                index = i,
                count = commits.rows.size,
                title = c.subject,
                supporting = c.detail,
                mono = true,
                trailing = if (c.selected) {
                    { ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary) }
                } else {
                    null
                },
                tag = "review-commit-$i",
            ) {
                runCatching { handle.pickReviewCommit(c.sha, c.subject) }
                    .onFailure { extras.model.toast(SessionExtras.failure(it, "Couldn't pick the commit. Try again.")) }
                onBack()
            }
        }
        item { Box(Modifier.navigationBarsPadding().height(16.dp)) }
    }
}

@Composable
private fun ModelsPage(extras: SessionExtras, harness: String, revision: Int, onBack: () -> Unit) {
    val handle = extras.handle
    var local by remember { mutableIntStateOf(0) }
    LaunchedEffect(harness) { runCatching { handle.loadReviewerModels(harness) } }
    val models = remember(revision, local, harness) { runCatching { handle.reviewerModels(harness) }.getOrNull() }
    fun pick(row: ReviewerOptionRow) {
        runCatching { handle.setReviewer(row.pick) }.onFailure { extras.model.toast(SessionExtras.failure(it, "Couldn't change the reviewer. Try again.")) }
        local++
    }
    LazyColumn(Modifier.fillMaxWidth().testTag("review-models")) {
        item { SheetTitle(models?.title ?: "Model", onBack) }
        if (models == null || (models.loading && models.models.isEmpty())) {
            item { Spinner() }
        } else {
            item { Box(Modifier.height(8.dp)) }
            optionRows(models.models, "review-model-0") { pick(it) }
            if (models.thinking.isNotEmpty()) {
                item { SectionHeader(models.thinkingHeading) }
                optionRows(models.thinking, "review-model-1") { pick(it) }
            }
        }
        item { Box(Modifier.navigationBarsPadding().height(16.dp)) }
    }
}

private fun LazyListScope.optionRows(rows: List<ReviewerOptionRow>, tag: String, onPick: (ReviewerOptionRow) -> Unit) {
    itemsIndexed(rows) { i, row ->
        OptionRow(
            index = i,
            count = rows.size,
            title = row.label,
            trailing = if (row.selected) {
                { ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.primary) }
            } else {
                null
            },
            tag = "$tag-$i",
        ) { onPick(row) }
    }
}

@Composable
internal fun SheetTitle(title: String, onBack: (() -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().padding(start = if (onBack == null) 20.dp else 4.dp, end = 20.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) IconButton(onClick = onBack) { ZIcon(ZIcons.Back, "Back") }
        Text(title, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Spinner() {
    Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { LoadingIndicator() }
}

@Composable
private fun Check(selected: Boolean) {
    ZIcon(ZIcons.Check, if (selected) "Selected" else null, Modifier.size(20.dp), tint = if (selected) MaterialTheme.colorScheme.primary else Color.Transparent)
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun OptionRow(
    index: Int,
    count: Int,
    title: String,
    supporting: String? = null,
    enabled: Boolean = true,
    mono: Boolean = false,
    leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
    tag: String,
    onClick: () -> Unit,
) {
    SegmentedListItem(
        onClick = onClick,
        enabled = enabled,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        leadingContent = leading,
        trailingContent = trailing,
        supportingContent = supporting?.let {
            {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = if (mono) GeistMono else null,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        },
        modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap).testTag(tag),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.outline,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
