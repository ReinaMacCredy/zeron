package sh.zeron.android.ui

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.zIndex
import sh.zeron.android.R
import uniffi.zeron_core.pastedTextPreview
import uniffi.zeron_core.predictionPlaceholder
import uniffi.zeron_core.quoteChipLabel

/**
 * The composer, shared by sessions and the new-session page. One tonal
 * surface with two states: a resting one-line capsule — [+] Message… [↑] —
 * and, while focused or holding a draft, a card with staged photos and the
 * full-width prompt above a toolbar of context chips.
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
fun ComposerSurface(
    model: ComposerModel,
    placeholder: String,
    action: ComposerAction,
    onAction: () -> Unit,
    modifier: Modifier = Modifier,
    alwaysCard: Boolean = false,
    attach: Boolean = true,
    focusRequester: FocusRequester = remember { FocusRequester() },
    prediction: String? = null,
    onFocusChange: (Boolean) -> Unit = {},
    chips: @Composable RowScope.() -> Unit = {},
) {
    var focused by remember { mutableStateOf(false) }
    var attachOpen by remember { mutableStateOf(false) }
    var reopenKeyboard by remember { mutableStateOf(false) }
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    val focusManager = androidx.compose.ui.platform.LocalFocusManager.current
    val imeUp = WindowInsets.isImeVisible
    sh.zeron.android.ui.composer.TrackKeyboardHeight()
    fun closeAttach() {
        attachOpen = false
        if (reopenKeyboard) {
            focusRequester.requestFocus()
            keyboard?.show()
        }
    }
    fun toggleAttach() {
        if (attachOpen) return closeAttach()
        reopenKeyboard = imeUp
        focusManager.clearFocus()
        attachOpen = true
    }
    val card = alwaysCard || focused || attachOpen || model.images.isNotEmpty() || model.pasted.isNotEmpty() || model.quotes.isNotEmpty()
    val ghost = predictionPlaceholder(prediction, model.text)
    Column(modifier.fillMaxWidth()) {
    Surface(
        shape = RoundedCornerShape(28.dp),
        color = composerContainer(),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.animateContentSize(MaterialTheme.motionScheme.defaultSpatialSpec())) {
            AnimatedVisibility(model.staged > 0 || model.quotes.isNotEmpty()) { StagedThumbnails(model) }
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 56.dp)) {
                if (!card) {
                    Spacer(Modifier.width(4.dp))
                    AttachToggle(attachOpen, attach) { toggleAttach() }
                } else {
                    Spacer(Modifier.width(20.dp))
                }
                Box(Modifier.weight(1f).padding(vertical = if (card) 16.dp else 8.dp)) {
                    if (model.text.isEmpty()) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                ghost ?: placeholder,
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f, fill = false).testTag("composer-placeholder"),
                            )
                            if (ghost != null && prediction != null) {
                                Spacer(Modifier.width(6.dp))
                                Surface(
                                    onClick = {
                                        model.setText(prediction)
                                        focusRequester.requestFocus()
                                    },
                                    shape = RoundedCornerShape(50),
                                    color = chipContainer(),
                                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.zIndex(1f).testTag("composer-accept-prediction"),
                                ) { Text("Accept", style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp)) }
                            }
                        }
                    }
                    val accent = MaterialTheme.colorScheme.primary
                    BasicTextField(
                        model.value,
                        { if (!model.stagesPaste(it)) model.value = it },
                        textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                        visualTransformation = remember(model, accent) { MentionTint(model, accent) },
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        maxLines = if (card) 8 else 2,
                        modifier = Modifier
                            .fillMaxWidth()
                            .focusRequester(focusRequester)
                            .onFocusChanged {
                                focused = it.isFocused
                                if (it.isFocused && attachOpen) attachOpen = false
                                onFocusChange(it.isFocused)
                            }
                            // Hardware keyboards: Ctrl/⌘+Enter sends (plain Enter is a newline).
                            .onPreviewKeyEvent { e ->
                                val send = e.type == androidx.compose.ui.input.key.KeyEventType.KeyDown &&
                                    e.key == androidx.compose.ui.input.key.Key.Enter && (e.isCtrlPressed || e.isMetaPressed)
                                if (send && action != ComposerAction.Stop && model.hasContent) onAction()
                                send
                            },
                    )
                }
                if (!card) {
                    Spacer(Modifier.width(8.dp))
                    ActionButton(action, model.hasContent, onAction)
                    Spacer(Modifier.width(8.dp))
                } else {
                    Spacer(Modifier.width(16.dp))
                }
            }
            AnimatedVisibility(card, enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(start = 4.dp, end = 8.dp, bottom = 8.dp),
                ) {
                    AttachToggle(attachOpen, attach) { toggleAttach() }
                    Row(
                        Modifier
                            .weight(1f)
                            .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                            .drawWithContent {
                                drawContent()
                                // Chips scrolling under the send button fade out.
                                val fade = ChipFade.toPx()
                                drawRect(
                                    Brush.horizontalGradient(listOf(Color.Black, Color.Transparent), startX = size.width - fade, endX = size.width),
                                    topLeft = Offset(size.width - fade, 0f),
                                    blendMode = BlendMode.DstIn,
                                )
                            }
                            .horizontalScroll(rememberScrollState())
                            .padding(end = ChipFade),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        content = chips,
                    )
                    Spacer(Modifier.width(8.dp))
                    ActionButton(action, model.hasContent, onAction)
                }
            }
        }
    }
    if (attachOpen) sh.zeron.android.ui.composer.AttachPanel(model, onClose = ::closeAttach)
    }
}

val ChipFade = 24.dp

private class MentionTint(private val model: ComposerModel, private val accent: Color) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val tokens = model.mentionTokens
        if (tokens.isEmpty()) return TransformedText(text, OffsetMapping.Identity)
        val style = SpanStyle(color = accent, fontWeight = FontWeight.Medium)
        val styled = AnnotatedString.Builder(text).apply {
            for (token in tokens) {
                var at = text.text.indexOf(token)
                while (at >= 0) {
                    addStyle(style, at, at + token.length)
                    at = text.text.indexOf(token, at + token.length)
                }
            }
        }.toAnnotatedString()
        return TransformedText(styled, OffsetMapping.Identity)
    }
}

@Composable
private fun StagedThumbnails(model: ComposerModel) {
    Row(
        Modifier.horizontalScroll(rememberScrollState()).padding(start = 12.dp, end = 12.dp, top = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (model.quotes.isNotEmpty()) QuoteChip(model)
        for (text in model.pasted) {
            val info = remember(text.id) { pastedTextPreview(text.name, text.text) }
            var open by remember { mutableStateOf(false) }
            Box(Modifier.size(width = 168.dp, height = 72.dp)) {
                Surface(onClick = { open = true }, shape = RoundedCornerShape(16.dp), color = chipContainer(), modifier = Modifier.fillMaxSize().testTag("composer-pasted-text")) {
                    Row(Modifier.padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        ZIcon(ZIcons.Text, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.width(9.dp))
                        Column {
                            Text(info.title, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(info.subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                        }
                    }
                }
                RemoveBadge("Remove pasted text", Modifier.align(Alignment.TopEnd)) { model.pasted.remove(text) }
            }
            if (open) PastedTextSheet(info.title, text.text) { open = false }
        }
        var previewing by remember { mutableStateOf<StagedImage?>(null) }
        previewing?.let { img ->
            val i = model.images.indexOf(img)
            if (i >= 0) sh.zeron.android.ui.composer.AttachmentPreview(img, i, model.images.size, onRemove = { model.images.remove(img) }, onDismiss = { previewing = null })
        }
        for (image in model.images) {
            Box(Modifier.size(72.dp)) {
                Image(image.thumb, "Attached image", Modifier.fillMaxSize().clip(RoundedCornerShape(16.dp)).clickable { previewing = image }.testTag("composer-image"), contentScale = ContentScale.Crop)
                RemoveBadge("Remove image", Modifier.align(Alignment.TopEnd)) { model.images.remove(image) }
            }
        }
    }
}

@Composable
private fun RemoveBadge(label: String, modifier: Modifier, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        shape = CircleShape,
        color = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.85f),
        contentColor = MaterialTheme.colorScheme.inverseOnSurface,
        modifier = modifier.padding(4.dp).size(22.dp),
    ) {
        Box(contentAlignment = Alignment.Center) { ZIcon(ZIcons.Close, label, Modifier.size(14.dp)) }
    }
}

@Composable
private fun QuoteChip(model: ComposerModel) {
    var open by remember { mutableStateOf(false) }
    Box {
        Surface(onClick = { open = true }, shape = RoundedCornerShape(50), color = chipContainer(), modifier = Modifier.testTag("composer-quotes")) {
            Row(Modifier.heightIn(min = 34.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                ZIcon(R.drawable.zi_wrap_text, null, Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(quoteChipLabel(model.quotes.size.toUInt()), style = MaterialTheme.typography.labelLarge)
            }
        }
        DropdownMenuPopup(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuGroup(shapes = MenuDefaults.groupShape(0, 1)) {
                model.quotes.forEachIndexed { i, q ->
                    DropdownMenuItem(onClick = {}, enabled = false, text = { Text(q, maxLines = 3, overflow = TextOverflow.Ellipsis) }, shape = MenuDefaults.itemShape(i, model.quotes.size + 1).shape)
                }
                DropdownMenuItem(
                    onClick = {
                        open = false
                        model.quotes.clear()
                    },
                    text = { Text("Remove", color = MaterialTheme.colorScheme.error) },
                    leadingIcon = { ZIcon(ZIcons.Close, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.error) },
                    shape = MenuDefaults.itemShape(model.quotes.size, model.quotes.size + 1).shape,
                )
            }
        }
    }
}

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun PastedTextSheet(title: String, text: String, onDismiss: () -> Unit) {
    androidx.compose.material3.ModalBottomSheet(onDismissRequest = onDismiss) {
        Text(title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp))
        Text(
            text,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = GeistMono,
            modifier = Modifier.fillMaxWidth().heightIn(max = 520.dp).verticalScroll(rememberScrollState()).padding(24.dp),
        )
    }
}

@Composable
private fun AttachToggle(open: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val turn by androidx.compose.animation.core.animateFloatAsState(
        if (open) 45f else 0f,
        androidx.compose.animation.core.spring(dampingRatio = 0.72f, stiffness = androidx.compose.animation.core.Spring.StiffnessMediumLow),
        label = "attach",
    )
    IconButton(onClick = onClick, enabled = enabled, modifier = Modifier.testTag("composer-attach")) {
        ZIcon(ZIcons.Plus, if (open) "Close attachments" else "Attach", Modifier.size(22.dp).graphicsLayer { rotationZ = turn }, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** An expressive choice menu: one group, inline section labels, a trailing check on the choice. */
@Composable
fun ChoiceMenu(expanded: Boolean, onDismiss: () -> Unit, sections: List<MenuSection>) {
    DropdownMenuPopup(expanded = expanded, onDismissRequest = onDismiss) {
        val groups = sections.filter { it.choices.isNotEmpty() }
        val all = groups.sumOf { it.choices.size }
        DropdownMenuGroup(shapes = MenuDefaults.groupShape(0, 1)) {
            var index = 0
            groups.forEach { section ->
                section.title?.let { title -> MenuDefaults.Label { Text(title, style = MaterialTheme.typography.labelMedium) } }
                section.choices.forEach { choice ->
                    DropdownMenuItem(
                        selected = choice.selected,
                        onClick = {
                            choice.onClick()
                            onDismiss()
                        },
                        text = { Text(choice.label) },
                        supportingText = choice.supporting?.let { { Text(it) } },
                        shapes = MenuDefaults.itemShape(index++, all),
                        leadingIcon = choice.leading,
                        selectedLeadingIcon = choice.leading,
                        trailingIcon = if (choice.selected) {
                            { ZIcon(ZIcons.Check, null, Modifier.size(20.dp)) }
                        } else {
                            null
                        },
                        colors = MenuDefaults.selectableItemColors(
                            selectedContainerColor = MaterialTheme.colorScheme.secondaryContainer,
                            selectedTextColor = MaterialTheme.colorScheme.onSecondaryContainer,
                            selectedLeadingIconColor = MaterialTheme.colorScheme.onSecondaryContainer,
                            selectedTrailingIconColor = MaterialTheme.colorScheme.onSecondaryContainer,
                        ),
                    )
                }
            }
        }
    }
}
