package sh.zeron.android.ui.git

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.GitModel
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.SectionHeader
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentShape
import uniffi.zeron_core.ColorRole
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.PrBoardView
import uniffi.zeron_core.PrFilter
import uniffi.zeron_core.PrPersonalView
import uniffi.zeron_core.PrRow
import uniffi.zeron_core.PrSortField

@Composable
fun GitNoticeList(title: String, body: String) {
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 12.dp, bottom = 32.dp)) {
        item("notice") { NoticeCard(title, body) }
    }
}

@Composable
fun NoticeCard(title: String, body: String, modifier: Modifier = Modifier) {
    Group(listOf(GroupRow("notice") {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            if (body.isNotEmpty()) Text(body, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }), modifier)
}

@Composable
fun BoardPane(
    client: CoreClient,
    prefs: GitModel,
    devicesEmpty: Boolean,
    deviceId: String?,
    repository: String?,
    repositoriesLoaded: Boolean,
    refresh: Int,
    onOpen: (String) -> Unit,
) {
    if (devicesEmpty) return GitNoticeList("No Mac serves pull requests", "Open Zeron on a Mac signed in to GitHub with gh, then refresh.")
    if (deviceId == null) return
    if (repository == null) {
        return if (repositoriesLoaded) GitNoticeList("No repository", "Add a project with a GitHub checkout on this Mac.") else GitNoticeList("Loading pull requests…", "")
    }
    val board = remember(client, deviceId, repository) { client.pullRequestBoard(deviceId, repository, prefs.stars) }
    var view by remember(board) { mutableStateOf<PrBoardView?>(null) }
    var loading by remember(board) { mutableStateOf(false) }
    var query by remember(board) { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    fun load(work: suspend () -> PrBoardView) {
        loading = true
        scope.launch {
            val next = work()
            if (next.deviceId == deviceId && next.repository == repository) {
                view = next
                loading = false
            }
        }
    }
    LaunchedEffect(board) { load { board.load(false) } }
    LaunchedEffect(board, refresh) { if (refresh > 0) load { board.load(true) } }

    val v = view
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 10.dp, bottom = 32.dp)) {
        item("controls") {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Segments(
                    listOf("Authored", "Reviewing", "All"),
                    when (v?.filter) { PrFilter.REVIEWING -> 1; PrFilter.ALL -> 2; else -> 0 },
                    { i -> load { board.setFilter(listOf(PrFilter.AUTHORED, PrFilter.REVIEWING, PrFilter.ALL)[i]) } },
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    val personal = v?.view ?: PrPersonalView.EVERYTHING
                    MenuPill(
                        personalLabel(personal),
                        PrPersonalView.entries.map { p -> MenuChoice(personalLabel(p), checked = p == personal) { view = board.setView(p) } },
                    )
                    val field = v?.sortField ?: PrSortField.UPDATED
                    val ascending = v?.ascending ?: false
                    MenuPill(
                        sortLabel(field),
                        PrSortField.entries.map { f -> MenuChoice(sortLabel(f), checked = f == field) { view = board.setSort(f, ascending) } } +
                            MenuChoice(if (ascending) "Newest first" else "Oldest first") { view = board.setSort(field, !ascending) },
                        trailing = if (ascending) ZIcons.ArrowUp else ZIcons.ArrowDown,
                    )
                }
                TextField(
                    query,
                    { query = it; view = board.setQuery(it) },
                    placeholder = { Text("Search pull requests") },
                    leadingIcon = { ZIcon(ZIcons.Search, null, Modifier.size(20.dp)) },
                    singleLine = true,
                    shape = RoundedCornerShape(50),
                    colors = TextFieldDefaults.colors(
                        focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent,
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    ),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
        when {
            v == null -> item("loading") { Box(Modifier.padding(top = 12.dp)) { NoticeCard("Loading pull requests…", "") } }
            v.error != null -> item("error") { Box(Modifier.padding(top = 12.dp)) { NoticeCard(v.error!!.title, v.error!!.body) } }
            v.empty != null -> item("empty") { Box(Modifier.padding(top = 12.dp)) { NoticeCard(v.empty!!, "") } }
            else -> {
                for (group in v.groups) {
                    item("group:${group.key}") { SectionHeader("${group.label} · ${group.rows.size}") }
                    itemsIndexed(group.rows, key = { _, r -> "row:${group.key}:${r.url}" }) { i, row ->
                        Box(Modifier.padding(horizontal = 12.dp).padding(bottom = 2.dp)) {
                            PrBoardRow(row, segmentShape(i, group.rows.size), onClick = { onOpen(row.url) }) {
                                view = board.setStars(prefs.toggleStar(row.starKey))
                            }
                        }
                    }
                }
                if (v.canLoadMore) {
                    item("more") {
                        Box(Modifier.padding(top = 12.dp)) {
                            Group(listOf(GroupRow("more", onClick = { load { board.loadMore() } }) {
                                Text(if (loading) "Loading…" else "Load more", color = MaterialTheme.colorScheme.primary, style = MaterialTheme.typography.titleSmall, modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                            }))
                        }
                    }
                }
                if (v.summary.isNotEmpty()) {
                    item("summary") {
                        Text(v.summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 28.dp, vertical = 10.dp))
                    }
                }
            }
        }
        item("pad") { Box(Modifier.navigationBarsPadding()) }
    }
}

private fun personalLabel(view: PrPersonalView) = when (view) {
    PrPersonalView.EVERYTHING -> "Everything"
    PrPersonalView.NEEDS_YOU -> "Needs you"
    PrPersonalView.STARRED -> "Starred"
}

private fun sortLabel(field: PrSortField) = when (field) {
    PrSortField.UPDATED -> "Recently updated"
    PrSortField.OPENED -> "Recently opened"
    PrSortField.CHANGES -> "Most changes"
}

@Composable
private fun PrBoardRow(row: PrRow, shape: androidx.compose.ui.graphics.Shape, onClick: () -> Unit, onStar: () -> Unit) {
    val state = rememberSwipeToDismissBoxState(confirmValueChange = { value ->
        if (value == SwipeToDismissBoxValue.EndToStart) onStar()
        false
    })
    val warning = rememberDiffPalette().color(ColorRole.WARNING)
    SwipeToDismissBox(
        state = state,
        enableDismissFromStartToEnd = false,
        backgroundContent = {
            Surface(Modifier.fillMaxSize(), shape = shape, color = warning) {
                Box(Modifier.padding(end = 24.dp), contentAlignment = Alignment.CenterEnd) {
                    Text(if (row.starred) "☆ Unstar" else "★ Star", color = Color.White, style = MaterialTheme.typography.labelLarge)
                }
            }
        },
    ) {
        Surface(onClick = onClick, shape = shape, color = cardColor(), modifier = Modifier.fillMaxWidth()) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                ZIcon(ZIcons.PullRequest, null, Modifier.size(22.dp), tint = row.glyphTone.color())
                Column(Modifier.weight(1f).padding(horizontal = 14.dp)) {
                    Text((if (row.starred) "★ " else "") + row.title, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    val statusColor = row.statusTone.color()
                    val muted = MaterialTheme.colorScheme.onSurfaceVariant
                    Text(
                        buildAnnotatedString {
                            withStyle(SpanStyle(color = statusColor)) { append(row.status) }
                            withStyle(SpanStyle(color = muted)) { append(" · #${row.number} · ${row.author}") }
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (row.ciLabel.isNotEmpty() || row.changes.isNotEmpty()) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            if (row.ciLabel.isNotEmpty()) Text(row.ciLabel, style = MaterialTheme.typography.labelMedium, color = row.ciTone.color())
                            if (row.changes.isNotEmpty()) Text(changesText(row.changes))
                        }
                    }
                }
                Text(row.updated, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
            }
        }
    }
}
