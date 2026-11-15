package su.libri

import android.content.ClipData
import android.media.AudioAttributes
import android.media.MediaPlayer
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import java.text.DateFormat
import java.time.OffsetDateTime
import java.util.Date
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import su.libri.core.drawnCard
import su.libri.core.personTurns
import su.libri.core.Chat
import su.libri.core.ClientException
import su.libri.core.Link
import su.libri.core.MemoryHealth
import su.libri.core.Message
import su.libri.core.Outbox
import su.libri.core.Quote
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import androidx.compose.material3.TextButton
import su.libri.core.Role

@Composable
fun LibriApp(model: ChatModel) {
    if (model.address == null) return ConnectScreen(model)
    // Every 30 seconds while on screen and at once on coming back, from here
    // because the panel's own checks run only while it is open.
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(model.host, owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.checkMemory()
                delay(30_000)
            }
        }
    }
    // A change of host starts every screen afresh, as a reload does the page.
    key(model.address) { Box(Modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize()) {
        MemoryLine(model.memory) { model.toMemory = true }
        // The line takes the status bar when it shows; Places' own padding
        // then leaves it alone.
        Box(Modifier.weight(1f).fillMaxWidth().then(if (model.memory?.state == "failing") Modifier.consumeWindowInsets(WindowInsets.statusBars) else Modifier)) {
            // A call whose tool draws a card opens it (Talk.kt ToolRow).
            val cards = model.cardTools
            val paper = Ink
            // A Libri page's link opens in the Library, in place; any other goes
            // to the browser, as before.
            val browser = androidx.compose.ui.platform.LocalUriHandler.current
            val links = remember(browser) {
                object : androidx.compose.ui.platform.UriHandler {
                    override fun openUri(uri: String) {
                        model.pageOf(uri)?.let { model.opening = it } ?: browser.openUri(uri)
                    }
                }
            }
            CompositionLocalProvider(
                LocalChatDetail provides model.chatDetail,
                LocalDisplayFile provides { name -> model.picture(name) },
                // A call's row opens its card full screen, drawn from what it
                // gave back (or, for show_card, asked afresh).
                LocalCards provides { t ->
                    val drawn = drawnCard(t.name, t.arguments)
                    if (drawn.name in cards) ({ model.callCardUrl(drawn, t, drawn.shown, true)?.let { model.fullCard = hostLook(it, paper) } }) else null
                },
                // A call's card draws under it; its message goes to the main
                // chat's composer unless the chat it sits in has its own.
                LocalCardHost provides CardHost(
                    cards = cards,
                    opens = model.cardOpens,
                    url = model::callCardUrl,
                    say = { words -> model.draft.text = joined(model.draft.text, words) },
                    full = { at -> model.fullCard = at },
                ),
                androidx.compose.ui.platform.LocalUriHandler provides links,
            ) { Places(model) }
        }
    }
    // Over the host in use, which stays as it was until another answers.
    if (model.adding) ConnectScreen(model)
    } }
}

/** The solid line above everything while the memory fails, the whole of it a
 * way to the detail. It has no close: it goes at the first status that isn't
 * failing, and a screen reader hears it once, as it comes. */
@Composable
private fun MemoryLine(memory: MemoryHealth?, detail: () -> Unit) {
    if (memory?.state != "failing") return
    val p = Ink
    Text(
        "Memory failing${memory.since?.let { " since ${stamp(it)}" }.orEmpty()}: ${memory.reason}",
        style = type(TypeScale.label, 1.4f, FontWeight.Medium), color = if (p.dark) p.paper else Color.White,
        maxLines = 1, overflow = TextOverflow.Ellipsis,
        modifier = Modifier.fillMaxWidth().background(p.accent).clickable(onClick = detail)
            .semantics { liveRegion = LiveRegionMode.Polite }
            .windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 14.dp, vertical = 8.dp),
    )
}

/** A moment as the app shows one: the time today, the date as well before. */
fun stamp(at: String): String {
    val moment = runCatching { OffsetDateTime.parse(at) }.getOrNull() ?: return at
    val date = Date.from(moment.toInstant())
    return if (DateUtils.isToday(date.time)) DateFormat.getTimeInstance(DateFormat.SHORT).format(date)
    else DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(date)
}

/** Asks for a host's address: the tailnet is the trust boundary, so an
 * address is all a phone needs. Opened to add a host, Back and Cancel go
 * back to the one in use; the first time there is none to go back to. */
@Composable
private fun ConnectScreen(model: ChatModel) {
    var typed by rememberSaveable { mutableStateOf("") }
    val go = { if (typed.isNotBlank() && !model.connecting) model.connect(typed) }
    val adding = model.address != null
    if (adding) BackHandler(onBack = model::cancelAdding)
    Surface(Modifier.fillMaxSize()) {
        if (adding) {
            Box(Modifier.statusBarsPadding().padding(8.dp)) {
                Round(onClick = model::cancelAdding, size = 36.dp) { Glyph(Glyph.chevronLeft, 18.dp, Ink.mid, label = "Back") }
            }
        }
        Column(
            Modifier.statusBarsPadding().imePadding().padding(28.dp),
            verticalArrangement = Arrangement.Center,
        ) {
            Text(if (adding) "Add a host" else "Libri", style = MaterialTheme.typography.displaySmall)
            Spacer(Modifier.height(8.dp))
            Text(
                "The address of the computer Libri runs on, such as my-computer:6780.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(24.dp))
            OutlinedTextField(
                value = typed,
                onValueChange = { typed = it },
                label = { Text("Address") },
                singleLine = true,
                isError = model.error != null,
                supportingText = model.error?.let { { Text(it) } },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { go() }),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(16.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = go, enabled = typed.isNotBlank() && !model.connecting) {
                    if (model.connecting) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    else Text(if (adding) "Add" else "Connect")
                }
                if (adding) TextButton(onClick = model::cancelAdding) { Text("Cancel") }
            }
        }
    }
}

fun linkText(chat: Chat?): String = when (val link = chat?.link) {
    null, is Link.Connecting -> "Connecting…"
    is Link.Live -> if (chat.typing) "Working" else "Connected"
    is Link.Down -> "Not reachable · ${link.reason}"
    is Link.SignedOut -> "Not signed in"
}

/** The chat: the talk, what waits on the person, and the composer, laid
 * out as the phone page: the talk 14px in from each side and 14px between
 * messages. It runs the whole screen, up under the status bar and the
 * floating header and down under the dock, the bar and the navigation bar,
 * all of which float over it, as the Claude app's; it fades out at the top
 * under the header, is clear down to the dock and dims beneath it
 * (chatFade), and leaves the dock's height at its foot, so its last message
 * scrolls whole above the composer. */
@Composable
fun ChatPlace(model: ChatModel, draft: Draft, pins: Pins, modifier: Modifier = Modifier) {
    val chat by model.chat.collectAsStateWithLifecycle()
    val c = chat
    val typing = c?.typing == true
    BoxWithConstraints(modifier) {
        val room = maxHeight
        val density = LocalDensity.current
        val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
        // The dock's height, the bar's room and the navigation bar's with it.
        val dock = remember { DockRoom() }
        val foot = buildList<Pair<String, @Composable () -> Unit>> {
            if (model.chatDetail != "simple" && c != null && typing && c.jobs.isNotEmpty()) add("jobs" to { Jobs(c.jobs) })
            c?.offer?.let { o ->
                // The list spaces items 14dp apart; the chips belong to the answer
                // above them, so this item is drawn 6dp higher and reports 6dp less
                // height, leaving the 8dp gap the page uses.
                add("replies/${o.ask ?: o.quote?.id}" to {
                    Box(Modifier.layout { m, k ->
                        val p = m.measure(k)
                        val up = 6.dp.roundToPx()
                        layout(p.width, (p.height - up).coerceAtLeast(0)) { p.place(0, -up) }
                    }) {
                        Replies(o) { r -> o.ask?.let { model.reply(it, true, mapOf(o.question.orEmpty() to r)) } ?: model.send(r, emptyList(), o.quote) }
                    }
                })
            }
        }
        val messages = c?.messages.orEmpty()
        ChatTalk(
            model, messages, rememberLazyListState(), model.follow, typing, draft,
            // Hidden under the status bar, fading in just below it.
            fadeHide = top,
            fadeTop = top + CHAT_TOP_FADE,
            // 24dp above the dock: 10 here and the 14 spacing the held blank,
            // first and 0 tall unless holding room, stands apart by.
            padding = PaddingValues(start = 14.dp, end = 14.dp, top = top + 72.dp, bottom = dock.under + 10.dp),
            spacing = 14.dp,
            under = dock.under,
            floor = dock.floor,
            waiting = c?.asks.orEmpty().isNotEmpty(),
            again = { text, last -> if (last) model.redo(text, "again") else model.send(text, emptyList()) },
            kept = pins,
            foot = foot,
        )
        ChatDock(dock) {
        // What waits on the person scrolls as one above the composer, in
        // what room the keyboard leaves, so the composer keeps its own.
        // Temp's drafts show in Temp.
        val cards = c?.cards.orEmpty().filterNot { it.scratchpad }
        // A question offered as replies under the message has no card.
        val asks = c?.asks.orEmpty().filter { it.id != c?.offer?.ask }
        if (asks.isNotEmpty() || cards.isNotEmpty()) {
            Column(
                Modifier.padding(horizontal = 14.dp).heightIn(max = room * 0.4f)
                    .verticalScroll(rememberScrollState()).padding(top = 10.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                asks.forEach { a -> AskCard(a) { yes, answers, always, failed -> model.reply(a.id, yes, answers, always = always, failed = failed) } }
                cards.forEach { c -> Letter(c) { yes -> model.decide(c.id, yes) } }
            }
        }
        val trouble = when (c?.link) {
            is Link.Down -> "The host does not answer; messages wait and go once it does."
            is Link.SignedOut -> "This host pairs its devices, which the app cannot do yet. Set open = true under [daemon] in its config."
            else -> null
        }
        Composer(model, draft, typing = typing, trouble = trouble, textMost = room * 0.3f)
        }
    }
}

/** A chat's dock as measured: its whole height ([under], the bar's and the
 * navigation bar's room with it) and the room under the composer ([floor]),
 * where the talk shows dimmed (Kit.kt chatFade). Measured, not summed from
 * the insets, so a page that keeps clear of a bar itself counts none. */
class DockRoom {
    internal var dockPx by mutableIntStateOf(0)
    internal var bodyPx by mutableIntStateOf(0)
    val under: Dp @Composable get() = with(LocalDensity.current) { dockPx.toDp() }
    // The composer's own 8dp margin at its foot.
    val floor: Dp @Composable get() = with(LocalDensity.current) { (dockPx - bodyPx).toDp() } + 8.dp
}

/** What floats over a chat's foot: what waits on the person and the
 * composer, above the bar and the navigation bar, the same in every chat. */
@Composable
fun BoxScope.ChatDock(room: DockRoom, content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.align(Alignment.BottomCenter).fillMaxWidth().onSizeChanged { room.dockPx = it.height }
            // The bar's room, which ends a gap above the bar, else (the
            // keyboard up) a gap; less the composer's own 8dp foot margin
            // (Composer.kt).
            .navigationBarsPadding().padding(bottom = LocalBarRoom.current.takeIf { it > 0.dp }.let { (it ?: FOOT_GAP) - 8.dp }),
    ) {
        Column(Modifier.fillMaxWidth().onSizeChanged { room.bodyPx = it.height }, content = content)
    }
}

/** Every chat's talk: the main chat's, Temp's and a thread's (App.tsx
 * ChatView). Newest at the bottom, where the list starts, so it stays in
 * view: a reversed list stands on its foot, so the dock growing (a draft, a
 * card) lifts the talk with it. One list, one follow, one way to draw a
 * message and one hold menu; a chat differs only by what it is given. Two
 * renderers, a hand-drawn one for the threads, drew a thread's log apart
 * from the chat's (2026-11-10). */
@Composable
fun ChatTalk(
    model: ChatModel,
    all: List<Message>,
    list: LazyListState,
    follow: Follow,
    typing: Boolean,
    /** Where Edit and Reply put words; none where the chat is only read. */
    draft: Draft?,
    /** The band under what floats at the top. */
    fadeTop: Dp,
    /** How far down the talk is wholly hidden: a status bar it runs under. */
    fadeHide: Dp = 0.dp,
    padding: PaddingValues,
    spacing: Dp,
    /** What floats over the talk's foot (the dock, with the bar's and the
     * navigation bar's room under it), which its buttons stand above. */
    under: Dp,
    /** The room under the composer, where the talk shows dimmed (DockRoom). */
    floor: Dp,
    /** A question waits on the person: that is no work, so no dots. */
    waiting: Boolean,
    /** Says the words again, the last answer's in its place if `last`; none
     * where the chat is only read. */
    again: ((text: String, last: Boolean) -> Unit)?,
    /** The chat hippo keeps, whose messages can be pinned and replied to. */
    kept: Pins? = null,
    /** Under the newest message, newest last: what works, offered replies. */
    foot: List<Pair<String, @Composable () -> Unit>> = emptyList(),
    /** Where nothing has been said yet. */
    empty: (@Composable () -> Unit)? = null,
) {
    val p = Ink
    // Answer-only presentation shows the person's words and finished answers.
    val messages = if (model.chatDetail != "simple") all else all.filter { m -> m.role == Role.USER || (m.role == Role.ASSISTANT && !m.progress && !m.live && !(typing && m == all.lastOrNull()) && m.text.isNotBlank()) }
    var viewing by remember { mutableStateOf<String?>(null) }
    var jumping by remember { mutableStateOf<ULong?>(null) }
    val acts = remember(model.host) {
        object : Acts {
            override fun picture(name: String) = model.picture(name)
            override fun view(url: String) { viewing = url }
            override fun hold(m: Message, bounds: Rect) { model.held = Held(m, bounds) }
            override fun resend(m: Message) = model.resend(m)
            override fun putBack(m: Message) = model.putBack(m)
            override fun jump(row: ULong) { jumping = row }
        }
    }
    follow.Keep(list, typing)
    val last = messages.lastOrNull()
    val streaming = last != null && last.live && (last.text.isNotBlank() || last.thinking.isNotBlank())
    val running = last?.live == true && last.tools.any { !it.done }
    val dots = typing && !waiting && (model.chatDetail == "simple" || (!streaming && !running))
    // A quote's tap: the list runs newest first, under the foot's items and
    // the dots when they show, as the LazyColumn below lays them out.
    LaunchedEffect(jumping) {
        val row = jumping ?: return@LaunchedEffect
        // Cleared once the scroll is done, so the same quote taps again: cleared
        // first, the key's change would cancel this effect mid-scroll.
        val i = messages.indexOfFirst { it.row == row }
        if (i < 0) {
            jumping = null
            return@LaunchedEffect
        }
        // The held blank at the very foot (Follow.foot) comes first.
        val lead = 1 + foot.size + (if (dots) 1 else 0)
        follow.pinned = false
        // At once, as the web's jump: an animated scroll over a long chat
        // ran past the 400 ms every interaction answers within.
        list.scrollToItem(lead + messages.size - 1 - i)
        jumping = null
    }
    // A card's message goes to this chat's composer, where it has one.
    val cardHost = LocalCardHost.current
    val cards = remember(cardHost, draft) { if (draft == null) cardHost else cardHost?.copy(say = { words -> draft.text = joined(draft.text, words) }) }
    // Whose turn made each answer: the person's own opens its cards.
    val byPerson = remember(all) { personTurns(all).toSet() }
    Box(Modifier.fillMaxSize()) { CompositionLocalProvider(LocalCardHost provides cards, LocalPersonTurns provides byPerson, LocalTalkList provides list) {
        TalkSize(model.textPx) {
            LazyColumn(
                // Under the dock the talk dims, beneath the composer's foot: its
                // 8dp margin, the bar's room and the navigation bar's. The fade
                // comes before follow.on, which places the list lowered by its
                // drift during a drag or fling: after it, the fade was drawn
                // in the lowered place and slid with the talk until it settled.
                Modifier.fillMaxSize()
                    .chatFade(Ink.paper, top = fadeTop, under = under, floor = floor, hide = fadeHide)
                    .then(follow.on(list)),
                state = list,
                reverseLayout = true,
                contentPadding = padding,
                verticalArrangement = Arrangement.spacedBy(spacing),
            ) {
                follow.foot(this)
                foot.asReversed().forEach { (key, draw) -> item(key = key) { draw() } }
                if (dots) item(key = "working") { Dots() }
                // Keyed by position too: the core's ids are unique in practice,
                // but a duplicate would end the app.
                items(messages.withIndex().reversed(), key = { (i, m) -> "$i/${m.id}" }, contentType = { (_, m) -> if (m.role == Role.USER) Follow.ASKED else m.role }) { (_, m) ->
                    Said(m, acts)
                }
                if (empty != null && messages.isEmpty()) item(key = "empty") { empty() }
            }
        }
        if (!follow.pinned) {
            Round(
                onClick = { follow.pinned = true },
                modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = under + 6.dp)
                    .shadow(4.dp, CircleShape).background(p.lift, CircleShape),
                edge = p.hair,
                size = 36.dp,
            ) { Glyph(Glyph.down, 16.dp, p.mid, label = "Scroll to bottom") }
        }
        ReadingStop(model, Modifier.align(Alignment.BottomEnd).padding(end = 14.dp, bottom = under + 6.dp))
    } }
    viewing?.let { Viewer(it) { viewing = null } }
    // One menu at a time, whichever chat holds it; each chat draws its own.
    model.held?.takeIf { h -> messages.any { it == h.message } }?.let { h ->
        MessageMenu(h, acts, model, draft, kept, typing, messages, again) { model.held = null }
    }
}

/** One message as the talk draws it. */
@Composable
fun Said(m: Message, acts: Acts) {
    val said = if (m.role == Role.USER) null else notice(m.text)
    when {
        said != null -> Notice(said)
        m.role == Role.USER -> Mine(m, acts)
        m.role == Role.ASSISTANT -> Theirs(m, acts) { acts.picture(it) }
        else -> Note(m)
    }
}

/** A message held: which, and where it stood on screen. */
data class Held(val message: Message, val bounds: Rect)

/** What holding something offers, as Claude's app does: the page dims and
 * blurs (Places blurs it), `copy` stands where the thing was, lifted on a
 * card if `framed`, and a 270dp menu hangs 10dp under it, left if framed and
 * right if not. One too tall to fit shows its top and scrolls. A tap outside
 * closes it (hold.tsx `HoldMenu`). */
@Composable
fun HoldMenu(
    bounds: Rect,
    framed: Boolean,
    items: List<Triple<String, ImageVector, () -> Unit>>,
    close: () -> Unit,
    caption: String? = null,
    copy: @Composable () -> Unit,
) {
    val p = Ink
    Popup(
        popupPositionProvider = WindowOrigin,
        onDismissRequest = close,
        properties = PopupProperties(focusable = true),
    ) {
        BoxWithConstraints(
            Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.3f)).pointerInput(Unit) { detectTapGestures { close() } },
        ) {
            val density = LocalDensity.current
            val screenH = constraints.maxHeight.toFloat()
            val screenW = constraints.maxWidth.toFloat()
            val px = { d: androidx.compose.ui.unit.Dp -> with(density) { d.toPx() } }
            val pad = if (framed) px(14.dp) else 0f
            val menuH = px((12 + 50 * items.size + (if (caption != null) 30 else 0)).dp)
            val menuW = minOf(px(270.dp), screenW - px(24.dp))
            val left = maxOf(px(12.dp), bounds.left - pad)
            val width = minOf(screenW - px(12.dp), bounds.right + pad) - left
            val height = minOf(bounds.height + 2 * pad, screenH - menuH - px(46.dp))
            val top = maxOf(px(16.dp), minOf(bounds.top - pad, screenH - height - menuH - px(26.dp)))
            val menuLeft = (if (framed) left else left + width - menuW).coerceIn(px(12.dp), screenW - menuW - px(12.dp))
            with(density) {
                Box(
                    Modifier.offset { IntOffset(left.toInt(), top.toInt()) }.size(width.toDp(), height.toDp())
                        .then(if (framed) Modifier.shadow(10.dp, RoundedCornerShape(Radii.xxl)).clip(RoundedCornerShape(Radii.xxl)).background(p.lift) else Modifier)
                        .verticalScroll(rememberScrollState())
                        .padding(if (framed) 14.dp else 0.dp),
                ) {
                    copy()
                }
                Column(
                    Modifier.offset { IntOffset(menuLeft.toInt(), (top + height + px(10.dp)).toInt()) }.width(menuW.toDp())
                        .shadow(12.dp, RoundedCornerShape(Radii.xxl)).clip(RoundedCornerShape(Radii.xxl)).background(p.lift)
                        .border(1.dp, p.frame, RoundedCornerShape(Radii.xxl)).pointerInput(Unit) { detectTapGestures { } }
                        .padding(vertical = 6.dp),
                ) {
                    // When the message was sent, and from which kind of device.
                    if (caption != null) Text(caption, style = Type.label, color = p.soft, modifier = Modifier.padding(start = 20.dp, end = 20.dp, top = 6.dp, bottom = 6.dp))
                    items.forEach { (label, icon, run) ->
                        Row(
                            Modifier.fillMaxWidth().height(50.dp).clickable(onClick = run).padding(horizontal = 20.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(label, style = type(TypeScale.prose, 1.3f), color = p.ink, modifier = Modifier.weight(1f))
                            Glyph(icon, 20.dp, p.mid)
                        }
                    }
                }
            }
        }
    }
}

/** The words for the kind of device a message came from (libri/src/origin.rs), as the page has them. */
private val DEVICE = mapOf(
    "android" to "Android app",
    "desktop" to "desktop app",
    "web-phone" to "phone browser",
    "web-desktop" to "browser",
    "other" to "another app",
)

/** Over a held message's menu: its time, and for the person's own the device it came from. */
private fun heldCaption(m: Message): String? {
    val time = m.date?.let { java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(it)) }
    val device = m.from?.let { DEVICE[it] }
    return listOfNotNull(time, device).joinToString(" · ").ifEmpty { null }
}

/** What holding a message offers. */
@Composable
private fun MessageMenu(
    h: Held,
    acts: Acts,
    model: ChatModel,
    draft: Draft?,
    pins: Pins?,
    typing: Boolean,
    messages: List<Message>,
    again: ((text: String, last: Boolean) -> Unit)?,
    close: () -> Unit,
) {
    val p = Ink
    val m = h.message
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var selecting by remember { mutableStateOf(false) }
    var promoting by remember { mutableStateOf(false) }
    val mine = m.role == Role.USER
    val said = m.words()
    val items = buildList<Triple<String, ImageVector, () -> Unit>> {
        if (said.isNotEmpty()) {
            add(Triple("Copy", Glyph.copy) {
                scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText("message", said))) }
                close()
            })
            add(Triple("Select text", Glyph.select) { selecting = true })
        }
        // Edit, the same in every chat: the words go into the box, to change
        // and send as a new message; the one edited stays as it was.
        if (draft != null && mine && m.text.isNotBlank() && m.outbox != Outbox.REFUSED && m.outbox != Outbox.HELD) add(Triple("Edit", Glyph.edit) { draft.quote = null; draft.text = m.text; draft.focus(); close() })
        if (pins != null && said.isNotEmpty()) add(Triple(if (pins.has(m)) "Unpin" else "Pin", Glyph.pin) { pins.toggle(m); close() })
        if (!mine && model.me?.voice != null && m.text.isNotBlank()) add(
            // The answer alone, as the page reads it: not the words on the way.
            if (model.reading?.text == m.text) Triple("Stop reading", Glyph.stop) { hush(model); close() }
            else Triple("Read aloud", Glyph.speak) { speak(model, m.text); close() },
        )
        if (again != null && !mine && !m.live) add(Triple("Redraw", Glyph.redraw) { redraw(messages, m, typing, again); close() })
        if (said.isNotEmpty() && !m.live) add(Triple("Create task", Glyph.promote) { promoting = true })
        // Only what hippo keeps can be named; a message still on its way has
        // no row.
        val row = m.row
        if (pins != null && draft != null && row != null && said.isNotEmpty() && !m.live) {
            val line = said.lineSequence().firstOrNull { it.isNotBlank() }.orEmpty().trim()
            add(Triple("Reply", Glyph.reply) { draft.quote = Quote(row, line); draft.focus(); close() })
        }
    }
    val noop = remember {
        object : Acts {
            override fun picture(name: String) = acts.picture(name)
            override fun view(url: String) {}
            override fun hold(m: Message, bounds: Rect) {}
            override fun resend(m: Message) {}
            override fun putBack(m: Message) {}
        }
    }
    HoldMenu(h.bounds, framed = !mine, items, close, caption = heldCaption(m)) { TalkSize(model.textPx) { Said(m, noop) } }
    if (selecting) Sheet(close) { SelectionContainer { Text(said, style = Type.body, color = p.ink) } }
    if (promoting) {
        var directions by remember { mutableStateOf("") }
        Sheet(close) {
            Text("Create task", style = type(TypeScale.prose, 1.3f, FontWeight.SemiBold), color = p.ink)
            Small("A thread of its own on the Tasks page, with its own context.", Modifier.padding(top = 2.dp))
            Field(directions, { directions = it }, "Directions, if any (e.g. build it, research the options)", Modifier.padding(vertical = 12.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Pressable("Create task", filled = true) { model.send(promotion(directions, m.id, said), emptyList()); close() }
            }
        }
    }
}

/** Places a popup at the window's top left corner, so its content lays out
 * in the window's own coordinates. */
object WindowOrigin : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize) = IntOffset.Zero
}

/** A card over a dimmed page, as the page's dialogs: 14px corners, --lift,
 * framed in --edge, 12px in from the sides and a tenth of the way down. */
@Composable
fun Sheet(close: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { close() } }, contentAlignment = Alignment.TopCenter) {
            Column(
                Modifier.padding(top = 80.dp, start = 12.dp, end = 12.dp).fillMaxWidth().heightIn(max = 600.dp).card(Ink)
                    .pointerInput(Unit) { detectTapGestures { } }.verticalScroll(rememberScrollState()).padding(20.dp),
                content = content,
            )
        }
    }
}

/** A text field drawn as the page's: --paper inside a --hair frame, 12px corners. */
@Composable
fun Field(value: String, onChange: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier, lines: Int = 4, secret: Boolean = false) {
    val p = Ink
    BasicTextField(
        value = value,
        onValueChange = onChange,
        textStyle = (if (secret) type(TypeScale.caption, 1.5f, family = Faces.mono) else Type.small).copy(color = p.ink),
        cursorBrush = SolidColor(p.ink),
        maxLines = lines,
        visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = if (secret) KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false) else KeyboardOptions.Default,
        modifier = modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.xl)).background(p.paper).border(1.dp, p.hair, RoundedCornerShape(Radii.xl)).padding(horizontal = 12.dp, vertical = 9.dp),
        decorationBox = { inner ->
            Box {
                if (value.isEmpty()) Text(placeholder, style = Type.small, color = p.soft)
                inner()
            }
        },
    )
}

/** A pill-shaped button: ink filled, or framed in --frame. */
@Composable
fun Pressable(label: String, filled: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val p = Ink
    Box(
        Modifier.height(36.dp).clip(CircleShape)
            .background(if (filled) (if (enabled) p.ink else p.hair) else Color.Transparent)
            .then(if (filled) Modifier else Modifier.border(1.dp, p.frame, CircleShape))
            .clickable(enabled = enabled, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, style = type(TypeScale.control, 1.3f, FontWeight.Medium), color = if (filled) (if (enabled) p.paper else p.soft) else p.ink)
    }
}

/** Asks again the question that led to an answer: the last answer is
 * drawn afresh in place, an older one is asked again below. */
private fun redraw(messages: List<Message>, m: Message, typing: Boolean, again: (String, Boolean) -> Unit) {
    val i = messages.indexOfFirst { it.id == m.id }
    val question = messages.subList(0, maxOf(i, 0)).lastOrNull { it.role == Role.USER } ?: return
    again(question.text, !typing && messages.drop(i + 1).none { it.role == Role.USER })
}

private var player: MediaPlayer? = null

/** Moves on with each stop, so a playing that has been stopped, whose words
 * the host is still making, starts nothing when they arrive. */
@Volatile private var plays = 0

/** Words being read aloud; `loading` until the first sound. */
data class Reading(val text: String, val loading: Boolean = true)

/** Plays words in `voice` at the speed Settings picked, once any other has
 * stopped; `began` at the first sound, `done` when it ends or fails. */
private fun play(model: ChatModel, text: String, voice: String?, began: () -> Unit, done: () -> Unit) {
    val mine = plays
    val over = {
        if (plays == mine) {
            hush(model)
            done()
        }
    }
    var url: String? = null
    model.act(then = {
        val at = url
        if (at == null) {
            over()
        } else if (plays == mine) {
            try {
                player = MediaPlayer().apply {
                    setAudioAttributes(AudioAttributes.Builder().setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).setUsage(AudioAttributes.USAGE_MEDIA).build())
                    setDataSource(at)
                    setOnPreparedListener {
                        it.playbackParams = it.playbackParams.setSpeed(model.voiceSpeed)
                        it.start()
                        began()
                    }
                    setOnCompletionListener { over() }
                    setOnErrorListener { _, _, _ -> over(); true }
                    prepareAsync()
                }
            } catch (e: java.io.IOException) {
                model.notice = e.message
                over()
            }
        }
    }) {
        try {
            url = model.host?.speak(text, voice)
        } catch (e: ClientException) {
            over()
            throw e
        }
    }
}

/** Reads the words aloud, one thing at a time, until they end or `hush`. */
fun speak(model: ChatModel, text: String) {
    hush(model)
    val now = Reading(text)
    model.reading = now
    play(model, text, model.voice, began = { if (model.reading === now) model.reading = now.copy(loading = false) }, done = {})
}

/** A voice heard from Settings: no reading, so the talk offers no way to stop it. */
fun sample(model: ChatModel, text: String, voice: String, done: () -> Unit) {
    hush(model)
    play(model, text, voice, began = {}, done = done)
}

/** Stops what is being read aloud. */
fun hush(model: ChatModel) {
    plays++
    player?.release()
    player = null
    model.reading = null
}

/** Stops the reading from wherever the person has gone since starting it,
 * since the reply may be scrolled away: a pill at the right, over a place's
 * composer, its square pulsing until the first sound (App.tsx ReadingStop). */
@Composable
fun ReadingStop(model: ChatModel, modifier: Modifier = Modifier) {
    val now = model.reading ?: return
    val p = Ink
    val fade = if (now.loading) {
        rememberInfiniteTransition(label = "reading").animateFloat(
            initialValue = 1f,
            targetValue = 0.5f,
            animationSpec = infiniteRepeatable(tween(1000), RepeatMode.Reverse),
            label = "reading-square",
        ).value
    } else {
        1f
    }
    Row(
        modifier.height(36.dp).shadow(6.dp, CircleShape, ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
            .clip(CircleShape).background(p.lift).border(1.dp, p.frame, CircleShape)
            .clickable { hush(model) }.padding(start = 12.dp, end = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.size(10.dp).alpha(fade).clip(RoundedCornerShape(2.dp)).background(p.ink))
        Text("Stop reading", style = Type.note, color = p.ink)
    }
}

@Composable
fun Notice(model: ChatModel) {
    val said = model.notice ?: return
    LaunchedEffect(said) {
        kotlinx.coroutines.delay(4000)
        model.notice = null
    }
    Surface(
        color = MaterialTheme.colorScheme.inverseSurface,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(Radii.xl),
        modifier = Modifier.padding(16.dp).clickable { model.notice = null },
    ) {
        Text(said, color = MaterialTheme.colorScheme.inverseOnSurface, modifier = Modifier.padding(14.dp))
    }
}
