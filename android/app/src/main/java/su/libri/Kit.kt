package su.libri

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp

/** Pieces every screen draws with, each as the web app draws its own. */

/** A line of quiet text: 13px in --soft unless told otherwise. */
@Composable
fun Small(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified, style: TextStyle = Type.note) {
    Text(text, style = style, color = if (color == Color.Unspecified) Ink.soft else color, modifier = modifier)
}

/** A section's head: caption capitals, tracked wide, in --soft. */
@Composable
fun Head(text: String, modifier: Modifier = Modifier, color: Color = Color.Unspecified) =
    Text(text.uppercase(), style = Type.head, color = if (color == Color.Unspecified) Ink.soft else color, modifier = modifier)

/** The page's card: --lift, 1.25px --edge frame, 14px corners, a soft shadow. */
fun Modifier.card(p: Paper, radius: Dp = Radii.card, framed: Boolean = true) = this
    .shadow(if (p.dark) 10.dp else 6.dp, RoundedCornerShape(radius), ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
    .clip(RoundedCornerShape(radius))
    .background(p.lift)
    .then(if (framed) Modifier.border(1.25.dp, p.edge, RoundedCornerShape(radius)) else Modifier)

/** A menu as the page's: a --lift card framed in --edge, opening from what
 * was pressed. Its rows are `MenuRow`s. */
@Composable
fun PaperMenu(
    open: Boolean,
    close: () -> Unit,
    modifier: Modifier = Modifier,
    offset: DpOffset = DpOffset(0.dp, 8.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    val p = Ink
    DropdownMenu(
        expanded = open,
        onDismissRequest = close,
        offset = offset,
        shape = RoundedCornerShape(Radii.card),
        containerColor = p.lift,
        tonalElevation = 0.dp,
        shadowElevation = 8.dp,
        border = BorderStroke(1.25.dp, p.edge),
        modifier = modifier.widthIn(min = 176.dp),
        content = content,
    )
}

/** A row of a menu: an icon, its words, and a check when it is the one picked. */
@Composable
fun MenuRow(
    label: String,
    onClick: () -> Unit,
    icon: ImageVector? = null,
    lead: (@Composable () -> Unit)? = null,
    note: String? = null,
    picked: Boolean = false,
    on: Boolean = false,
) {
    val p = Ink
    Row(
        Modifier.fillMaxWidth().background(if (on) p.hover else Color.Transparent)
            .clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        CompositionLocalProvider(LocalContentColor provides p.ink) {
            lead?.invoke()
            icon?.let { Glyph(it, 17.dp) }
            Column(Modifier.weight(1f)) {
                Text(label, style = type(TypeScale.control, 1.35f), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                note?.let { Text(it, style = type(TypeScale.caption, 1.3f), color = p.soft) }
            }
            if (picked) Glyph(Glyph.check, 15.dp)
        }
    }
}

/** The one make of all that floats over a phone's page (
 * "consistent ui everywhere"; index.css floating): the bar, its disc and
 * plus, the controls at the top, Temp's chips and the composer share this
 * lift, hairline and soft shadow, round-ended and 14dp from the screen's
 * sides. The lift is opaque: blur behind is the web's alone. */
fun Modifier.floating(p: Paper, shape: Shape = CircleShape, edge: Boolean = true) =
    this.shadow(4.dp, shape, ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
        .clip(shape).background(p.lift).then(if (edge) Modifier.border(1.dp, p.hair, shape) else Modifier)

/** A floating control with words (Temp's Memory and Clear, its Promote
 * chips): the [floating] make, `height` tall. */
@Composable
fun FloatPill(height: Dp = 44.dp, onClick: (() -> Unit)?, modifier: Modifier = Modifier, content: @Composable RowScope.() -> Unit) {
    Row(
        modifier.height(height).floating(Ink).then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = if (height < 40.dp) 14.dp else 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        content = content,
    )
}

/** A round button of the header, floating over the talk: a 44dp [floating]
 * disc, as the page's DISC. */
@Composable
fun FloatButton(icon: ImageVector, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, badge: Boolean = false) =
    FloatButton(label, onClick, modifier, badge) { Glyph(icon, 18.dp, Ink.mid) }

/** The same, with a mark drawn its own way (Temp's dashed bubble). */
@Composable
fun FloatButton(label: String, onClick: () -> Unit, modifier: Modifier = Modifier, badge: Boolean = false, icon: @Composable () -> Unit) {
    val p = Ink
    Box(
        modifier.size(44.dp).floating(p)
            .clickable(onClickLabel = label, onClick = onClick)
            // Named on the button itself: a drawn mark (Temp's) carries no name of its own.
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        icon()
        if (badge) Box(Modifier.align(Alignment.TopEnd).padding(9.dp).size(7.dp).background(p.accent, CircleShape))
    }
}

/** How far the floating bar (Places.kt Bar) reaches up from the foot of a
 * place: a list lies under it and pads its foot by this, so its last row
 * scrolls clear, and a [Fab] stands this much higher. Zero while the
 * keyboard hides the bar, and on any page the bar is not over. */
val LocalBarRoom = androidx.compose.runtime.compositionLocalOf { 0.dp }

/** What scrolls under floating chrome (the bar, a header) fades out
 * instead of being cut off, as in the Claude and ChatGPT apps: a band of the
 * page's [paper] at each edge, clear inward. A scrim rather than an alpha
 * mask, since the places lie on plain paper and a mask would draw the whole
 * list offscreen each frame it scrolls. The list's padding at each edge
 * should be at least the band, so its first and last rows stay whole. */
fun Modifier.edgeFade(paper: Color, top: Dp = 0.dp, bottom: Dp = 0.dp) = drawWithCache {
    // Built once per size, not per frame: a scrolling list redraws every frame.
    val t = top.toPx()
    val b = bottom.toPx()
    val y = size.height - b
    val head = Brush.verticalGradient(0f to paper, 1f to paper.copy(alpha = 0f), startY = 0f, endY = t)
    val foot = Brush.verticalGradient(0f to paper.copy(alpha = 0f), 1f to paper, startY = y, endY = size.height)
    onDrawWithContent {
        drawContent()
        if (t > 0f) drawRect(head, size = Size(size.width, t))
        if (b > 0f) drawRect(foot, topLeft = Offset(0f, y), size = Size(size.width, b))
    }
}

/** How much of the paper lies over the talk beneath a chat's composer: the
 * talk shows through at 15%, as measured from a photo of Claude Duet's
 * (2026-11-11). index.css chat-edge-foot is the web's. */
const val FOOT_SCRIM = 0.85f

/** A chat's edges, as Claude Duet's: at the top the talk is hidden under
 * the status bar (down to [hide]) and fades in under the header's buttons
 * down to [top]; at the foot the talk stays wholly clear down to the
 * dock's top edge ([under] above the bottom), dims behind the composer, and
 * shows on at [FOOT_SCRIM] beneath it ([floor], the room under the
 * composer: the bar and the navigation bar). */
fun Modifier.chatFade(paper: Color, top: Dp, under: Dp, floor: Dp, hide: Dp = 0.dp) = drawWithCache {
    val h = hide.toPx()
    val t = top.toPx().coerceAtLeast(h)
    val u = under.toPx()
    val f = floor.toPx().coerceAtMost(u)
    val head = Brush.verticalGradient(0f to paper, 1f to paper.copy(alpha = 0f), startY = h, endY = t)
    val dim = paper.copy(alpha = FOOT_SCRIM)
    val from = size.height - u
    val to = size.height - f
    val ramp = Brush.verticalGradient(0f to paper.copy(alpha = 0f), 1f to dim, startY = from, endY = to)
    onDrawWithContent {
        drawContent()
        if (h > 0f) drawRect(paper, size = Size(size.width, h))
        if (t > h) drawRect(head, topLeft = Offset(0f, h), size = Size(size.width, t - h))
        if (to > from) drawRect(ramp, topLeft = Offset(0f, from), size = Size(size.width, to - from))
        if (f > 0f) drawRect(dim, topLeft = Offset(0f, to), size = Size(size.width, f))
    }
}

/** The status bar's height: a place runs up under it (
 * no band at the top), its list padded this much more so the first row
 * starts clear, and only [TOP_FADE] of paper fading out below the icons. */
val statusTop: Dp
    @Composable get() = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()

/** The thin fade under the status bar's icons (index.css --top-fade). */
val TOP_FADE = 8.dp

/** A chat's fade below the status bar: a short band, the talk clear behind
 * the header's buttons, as Claude Duet's (photo, 2026-11-13; phone.tsx
 * --chat-top-fade). */
val CHAT_TOP_FADE = 28.dp

/** Where a place's first line starts under the floating controls at the
 * top: 10dp off the status bar, 44dp tall, 10dp under (--chrome-top). */
val CHROME_TOP = 64.dp

/** How much a list under a [Fab] pads its foot, so its last row scrolls clear
 * of the button: the button and its gap, and a gap again. */
val FAB_CLEARANCE = 56.dp + 16.dp + 16.dp

/** The page's one floating action: a 56dp accent disc at the lower right of
 * its place, 16dp in from the edge and 16dp above the bar, Material 3's FAB
 * size and margins, where Duet floats its + and the thumb reaches. It turns
 * to a close mark while what it opened is open. Hidden while the keyboard is
 * up, as the bar is. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun BoxScope.Fab(label: String, open: Boolean, onClick: () -> Unit) {
    if (WindowInsets.isImeVisible) return
    val p = Ink
    // --on-accent: white by day, the paper by night (index.css).
    val on = if (p.dark) p.paper else Color.White
    Box(
        Modifier.align(Alignment.BottomEnd).padding(end = 16.dp, bottom = 16.dp + LocalBarRoom.current)
            .size(56.dp).shadow(6.dp, CircleShape, ambientColor = Color(0x33000000), spotColor = Color(0x33000000))
            .clip(CircleShape).background(p.accent)
            .clickable(onClickLabel = label, onClick = onClick)
            // Named on the button itself: a drawn mark (Temp's) carries no name of its own.
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Glyph(if (open) Glyph.close else Glyph.plus, 24.dp, on, label = if (open) "Close" else label)
    }
}

/** A plain round button: 38px, as the composer's. */
@Composable
fun Round(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    fill: Color = Color.Transparent,
    edge: Color = Color.Transparent,
    enabled: Boolean = true,
    size: Dp = 38.dp,
    content: @Composable () -> Unit,
) {
    Box(
        modifier.size(size).clip(CircleShape).background(fill)
            .then(if (edge != Color.Transparent) Modifier.border(1.dp, edge, CircleShape) else Modifier)
            .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** A settings row: words on the left, a value in --soft on the right. */
@Composable
fun SettingRow(label: String, value: String? = null, onClick: (() -> Unit)? = null, chevron: Boolean = onClick != null, trailing: (@Composable RowScope.() -> Unit)? = null) {
    val p = Ink
    Row(
        // 44 tall at the least, as the spec's phone rows.
        Modifier.fillMaxWidth().heightIn(min = 44.dp).then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 20.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.weight(1f))
        value?.let { Text(it, style = Type.note, color = p.soft, maxLines = 1) }
        trailing?.invoke(this)
        if (chevron) Glyph(Glyph.chevronRight, 15.dp, p.soft, Modifier.padding(start = 6.dp))
    }
}

/** A hairline across. */
@Composable
fun Rule(modifier: Modifier = Modifier) = Box(modifier.fillMaxWidth().height(1.dp).background(Ink.hair))
