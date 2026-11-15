package su.libri

import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.zIndex
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.role
import android.content.ClipData
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.draw.alpha
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.draw.scale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.RowScope
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import kotlin.math.abs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import su.libri.core.Builtin
import su.libri.core.Connections
import su.libri.core.Speech
import su.libri.core.Server
import su.libri.core.Tested
import su.libri.core.Guide
import su.libri.core.Host
import su.libri.core.Makers
import su.libri.core.MemoryHealth
import su.libri.core.ModelRef
import su.libri.core.Offer
import su.libri.core.SynopsisModel
import su.libri.core.Voice
import su.libri.core.WorkerModel
import su.libri.core.WorkerNames
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.runtime.key
import androidx.compose.ui.focus.focusRequester
import su.libri.core.PhonePane
import su.libri.core.PhoneClosed
import su.libri.core.PhoneSpace

/** The bar names Temp short; every other kind as the host does (tiles.rs). */
private fun kindName(model: ChatModel, kind: String) = if (kind == "temp") "Temp" else model.tiles?.of(kind)?.name ?: kind.replaceFirstChar { it.uppercase() }

/** One item, as the bar and the host key it: its kind, and which one. */
// A workspace has one Whiteboard item, its tabs inside it: the board it
// shows is its own state, not part of what it is, so picking a tab keeps it
// shown and keeps its web view rather than making a new item.
private fun PhonePane.key() = if (kind == "board") kind else kind + (arg?.let { ":$it" } ?: "")

/** The app's places (a design choice of 2026-11-10, as phone.tsx): the bar
 * at the foot is a workspace, a row of items the person adds to with the
 * plus and takes from by a long press; the pill left of it switches
 * workspace. A swipe on the page steps along the items, and past the first
 * opens the side panel; a swipe on the bar goes to the next workspace. The
 * workspaces are kept on the host (ChatModel.spaces), the same as the web's
 * on a phone. On the chat the side panel's button floats at the top left
 * and search at the top right. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun Places(model: ChatModel) {
    val p = Ink
    val context = LocalContext.current
    val pins = remember { Pins(context) }
    val draft = model.draft
    val ws = model.spaces.firstOrNull { it.id == model.spaceOn } ?: model.spaces.first()
    val items = ws.items
    // Which item shows is this phone's own, never kept on the host.
    var shownKey by rememberSaveable { mutableStateOf<String?>(null) }
    val at = items.indexOfFirst { it.key() == shownKey }.coerceAtLeast(0)
    val shown = items[at]
    // Something opened that is on no item of this workspace (a page asked
    // open, a worker's thread) lies over the item shown until put down.
    var over by rememberSaveable { mutableStateOf<String?>(null) }
    val show = { i: Int ->
        over = null
        items.getOrNull(i)?.let { shownKey = it.key() }
    }
    val showKind = { kind: String ->
        val i = items.indexOfFirst { it.kind == kind && it.arg == null }
        if (i >= 0) show(i) else over = kind
    }
    val toChat = { showKind("chat") }
    // A Temp tab made a Temp of its own: the tab holds it by its id from now.
    val holdTemp = { item: PhonePane, id: String ->
        val held = PhonePane("temp", id)
        model.changeSpaces(model.spaces.map { w -> if (w.id == ws.id) w.copy(items = w.items.map { if (it == item) held else it }) else w }, model.spaceOn)
        if (shownKey == item.key()) shownKey = held.key()
    }
    // A pane asked for as a tab of its own: added to the bar, and shown.
    LaunchedEffect(model.tabAsked) {
        val pane = model.tabAsked ?: return@LaunchedEffect
        if (items.none { it.key() == pane.key() }) model.changeSpaces(model.spaces.map { if (it.id == ws.id) it.copy(items = it.items + pane) else it }, model.spaceOn)
        over = null
        shownKey = pane.key()
        model.tabAsked = null
    }
    var searching by rememberSaveable { mutableStateOf(false) }
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(model.host, owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) { model.followSpaces() }
    }
    // A Library page asked open (ChatModel.opening) comes up in the Library.
    val asked = model.opening
    LaunchedEffect(asked) { if (asked != null) showKind("library") }
    // A thread opened from elsewhere (a notification) shows on the Work page,
    // laid over the item shown, as every worker is (the composer's Show all).
    LaunchedEffect(model.opened) { if (model.opened != null && over != "workers") over = "workers" }
    LaunchedEffect(model.allWorkersAsked) { if (model.allWorkersAsked > 0) over = "workers" }
    // The side panel lies under the page at its left; open, the page moves
    // over to show it and stays in view at the right edge as a card, as
    // Claude Duet's does (a design choice of 2026-11-06), rather than the panel covering it.
    // `slid` runs 0 (shut) to 1 (open) and follows a finger while dragged.
    var panelOpen by rememberSaveable { mutableStateOf(false) }
    val slid = remember { Animatable(if (panelOpen) 1f else 0f) }
    val scope = rememberCoroutineScope()
    val panel = { open: Boolean ->
        panelOpen = open
        scope.launch { slid.animateTo(if (open) 1f else 0f, tween(260, easing = FastOutSlowInEasing)) }
        Unit
    }
    LaunchedEffect(model.sharedPlace) {
        if (model.sharedPlace > 0) {
            model.fold()
            toChat()
        }
    }
    val tempShown = shown.kind == "temp" && over == null
    val shownTemp = if (tempShown) model.temp(tempKey(shown.arg)) else null
    LaunchedEffect(shownTemp) {
        model.temps.values.forEach { it.open = it === shownTemp }
        shownTemp?.unread = false
    }
    // The Work list, asked again the faster while anything works, wherever
    // the app is, as long as it is in view (work.tsx useWork).
    val busy = model.threads.any { it.state == "working" }
    LaunchedEffect(busy, owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.refreshWork()
                delay(if (busy) 4_000 else 15_000)
            }
        }
    }
    LaunchedEffect(model.host) { model.refreshBuiltins() }
    if (model.calling) {
        CallScreen(model) { model.calling = false }
        return
    }
    LaunchedEffect(model.host) { model.refreshCards() }
    model.fullCard?.let { at ->
        CardScreen(at) { model.fullCard = null }
        return
    }
    if (searching) {
        SearchScreen(model) { searching = false }
        return
    }
    if (model.newTask) {
        TaskSheet(model) { model.newTask = false }
        return
    }
    LetGoOfFields(shown.key() + over, panelOpen)
    LaunchedEffect(model.toConnections) { if (model.toConnections) panel(true) }
    LaunchedEffect(model.toMemory) {
        if (model.toMemory) { panel(true); model.toMemory = false }
    }
    // What lies open over the bar: the workspaces or the plus's menu.
    var menu by remember { mutableStateOf<String?>(null) }
    // The bar being edited, an item on its way off it, and the last one
    // closed, for its Undo (Bar). The chat never closes: the bar is the
    // way back to it.
    var editing by remember { mutableStateOf(false) }
    var leaving by remember { mutableStateOf<String?>(null) }
    var closed by remember { mutableStateOf<ClosedItem?>(null) }
    LaunchedEffect(closed) { if (closed != null) { delay(5_000); closed = null } }
    LaunchedEffect(ws.id) { editing = false }
    val firstChat = items.indexOfFirst { it.kind == "chat" }
    // A chat item's memories as tabs (MemoryTabs.kt): a change keeps the
    // item, unless the memory it would show is another item's, which then
    // comes forward, as the bar holds each once.
    val setChat = { item: PhonePane, pane: PhonePane ->
        val other = items.indexOfFirst { it != item && it.key() == pane.key() }
        if (other >= 0) shownKey = items[other].key()
        else {
            model.changeSpaces(model.spaces.map { w -> if (w.id == ws.id) w.copy(items = w.items.map { if (it == item) pane else it }) else w }, model.spaceOn)
            shownKey = pane.key()
        }
    }
    // A memory asked open (the Workers tree, `/memory <name>`): its tab in
    // the chat shown, else the first chat, opens or comes forward.
    LaunchedEffect(model.memoryTabAsked) {
        val id = model.memoryTabAsked ?: return@LaunchedEffect
        model.memoryTabAsked = null
        val item = items.getOrNull(at)?.takeIf { it.kind == "chat" } ?: items.getOrNull(firstChat) ?: return@LaunchedEffect
        over = null
        val tabs = item.memoryTabs()
        setChat(item, chatPane(id, if (id in tabs) tabs else tabs + id))
    }
    // One handler for all: registered apart, a later one would take Back first.
    BackHandler(enabled = editing || menu != null || panelOpen || over != null || model.opened != null || at != firstChat.coerceAtLeast(0)) {
        when {
            editing -> editing = false
            menu != null -> menu = null
            panelOpen -> panel(false)
            over == "workers" && model.opened != null -> model.fold()
            over != null -> over = null
            else -> show(firstChat.coerceAtLeast(0))
        }
    }
    val swipe = { right: Boolean ->
        when {
            right && at == 0 && over == null -> panel(true)
            right -> show((at - 1).coerceAtLeast(0))
            at < items.lastIndex -> show(at + 1)
        }
        Unit
    }
    val goSpace = { by: Int ->
        val i = model.spaces.indexOfFirst { it.id == ws.id } + by
        model.spaces.getOrNull(i)?.let { model.changeSpaces(model.spaces, it.id); shownKey = null; over = null }
        Unit
    }
    val change = { f: (PhoneSpace) -> PhoneSpace -> model.changeSpaces(model.spaces.map { if (it.id == ws.id) f(it) else it }, model.spaceOn) }
    val closable = { item: PhonePane -> !(item.kind == "chat" && item.arg == null) && items.size > 1 }
    val moveItem = { from: Int, to: Int ->
        if (from != to && from in items.indices && to in items.indices) change { w -> w.copy(items = w.items.toMutableList().also { it.add(to, it.removeAt(from)) }) }
    }
    val closeItem = { i: Int ->
        val gone = items.getOrNull(i)
        if (gone != null && closable(gone) && leaving == null) {
            // A Temp's tab closed ends that Temp; an Undo brings a fresh one.
            if (gone.kind == "temp" && gone.arg != null && gone.arg != "new") model.act { model.host?.scratchpadClose(gone.arg) }
            // It rises and fades while its room closes (Bar), then goes.
            leaving = gone.key()
            scope.launch {
                delay(180)
                val now = model.spaces.firstOrNull { it.id == ws.id }?.items.orEmpty()
                val j = now.indexOfFirst { it.key() == gone.key() }
                if (j >= 0) {
                    if (shownKey == gone.key() || (shownKey == null && j == 0)) shownKey = (now.getOrNull(j - 1) ?: now.getOrNull(j + 1))?.key()
                    change { w -> w.copy(items = w.items.filter { it.key() != gone.key() }) }
                    closed = ClosedItem(ws.id, gone, j, itemName(model, gone))
                }
                leaving = null
            }
        }
        Unit
    }
    // An item's menu, from a hold on it in the bar (TileViews.kt ItemMenu).
    var itemMenu by remember { mutableStateOf<Int?>(null) }
    /** Puts item `i` on workspace `to`'s bar (a new one when none is
     * named), off this one, and goes there. */
    val moveTo = { i: Int, to: String? ->
        items.getOrNull(i)?.let { p ->
            val target = to ?: freshSpaceId()
            val moved = model.spaces.map { w ->
                when {
                    w.id == ws.id -> w.copy(items = w.items.filterIndexed { j, _ -> j != i })
                    w.id == target && w.items.none { it.key() == p.key() } -> w.copy(items = w.items + p)
                    else -> w
                }
            } + if (to == null) listOf(PhoneSpace(target, unnamedSpace(model.spaces.map { it.name }), listOf(PhonePane("chat", null), p))) else emptyList()
            model.changeSpaces(moved, target)
            shownKey = p.key()
        }
        Unit
    }
    // The code behind each row the host names (tiles.rs), for item `i`.
    val itemCode = { i: Int, item: PhonePane ->
        val arg = item.arg
        val others = model.spaces.filter { it.id != ws.id }
        val pageUrl = { frag: String -> (model.address ?: "").trimEnd('/') + "/#" + frag }
        val view = { url: String -> runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url))) } }
        val lastAnswer = model.chat.value?.messages?.lastOrNull { it.role == su.libri.core.Role.ASSISTANT && !it.progress && it.text.isNotBlank() }?.text
        val own: Map<String, RowCode?> = when (item.kind) {
            "chat" -> mapOf(
                "search" to RowCode { searching = true },
                "copy-answer" to RowCode(enabled = lastAnswer != null) {
                    (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager)
                        .setPrimaryClip(android.content.ClipData.newPlainText("answer", lastAnswer))
                },
                "new-temp" to RowCode { val t = another(model.tiles, PhonePane("temp", null), items); if (items.none { it.key() == t.key() }) change { it.copy(items = it.items + t) }; over = null; shownKey = t.key() },
            )
            "temp" -> mapOf("promote" to RowCode { model.temp(tempKey(arg)).promote(true) })
            "terminal" -> model.terminalActions[arg ?: ""].let { t ->
                mapOf(
                    "paste" to RowCode(enabled = t != null) {
                        val clip = (context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager).primaryClip
                        clip?.getItemAt(0)?.coerceToText(context)?.let { t?.paste?.invoke(it.toString()) }
                    },
                    "clear" to RowCode(enabled = t != null) { t?.clear?.invoke() },
                    "restart" to RowCode(enabled = t != null) { t?.restart?.invoke() },
                )
            }
            "thread" -> mapOf(
                "stop" to RowCode(enabled = model.threads.any { it.id == arg && it.state == "working" }) { model.act { arg?.let { model.host?.agentStop(it) } } },
                "archive" to RowCode { arg?.let { model.workAct(it, "archive") } },
            )
            "library" -> if (arg?.startsWith("page:") == true) mapOf(
                "reload" to RowCode { model.libraryDrawn++ },
                "window" to RowCode { view(pageUrl("page=" + android.net.Uri.encode(arg.removePrefix("page:")))) },
            ) else emptyMap()
            "board" -> mapOf(
                "new-board" to RowCode { change { w -> w.copy(items = w.items.map { if (it == item) PhonePane("board", "new") else it }) }; shownKey = PhonePane("board", "new").key() },
                "export" to RowCode { view(pageUrl("board")) },
            )
            else -> emptyMap()
        }
        own + mapOf(
            "move" to if (closable(item)) RowCode(then = others.map { w -> w.name to { moveTo(i, w.id) } } + listOf("New workspace" to { moveTo(i, null) })) else null,
            "edit-bar" to RowCode { editing = true },
            "close" to if (closable(item)) RowCode { closeItem(i) } else null,
        )
    }
    val density = LocalDensity.current
    // The panel's width, known only inside BoxWithConstraints; the drag
    // below reads it from here.
    val reach = remember { floatArrayOf(1f) }
    // While open, a drag either way moves panel and page together under the
    // finger; let go, they settle by the fling, or else by how far. On the
    // whole, after its children: the panel's scroll and taps go first, and
    // this takes only what moves sideways past the touch slop.
    BoxWithConstraints(
        Modifier.fillMaxSize().background(p.paper).pointerInput(panelOpen) {
            if (!panelOpen) return@pointerInput
            val tracker = VelocityTracker()
            detectHorizontalDragGestures(
                onDragStart = { tracker.resetTracking() },
                onDragEnd = {
                    val v = tracker.calculateVelocity().x
                    panel(if (abs(v) > 600f) v > 0f else slid.value > 0.5f)
                },
                onDragCancel = { panel(slid.value > 0.5f) },
            ) { change2, by ->
                change2.consume()
                tracker.addPosition(change2.uptimeMillis, change2.position)
                scope.launch { slid.snapTo((slid.value + by / reach[0]).coerceIn(0f, 1f)) }
            }
        },
    ) {
    // The page keeps 56dp in view beside the panel: enough to see it is
    // there and to tap it shut.
    val panelWidth = minOf(340.dp, maxWidth - 56.dp)
    val travel = with(density) { panelWidth.toPx() }
    reach[0] = travel
    val corner = with(density) { 28.dp.toPx() }
    val line = with(density) { 1.25.dp.toPx() }
    // Drawn only while any of it shows, so a shut panel asks nothing.
    if (panelOpen || slid.value > 0f) {
        Box(
            Modifier.width(panelWidth).fillMaxHeight()
                // It comes in a little behind the page, so the two read as layers.
                .graphicsLayer { translationX = -(1f - slid.value) * travel * 0.25f },
        ) {
            SidePanel(model, pins, open = panelOpen) { panel(false) }
        }
    }
    Box(
        Modifier.fillMaxSize()
            .graphicsLayer {
                translationX = slid.value * travel
                shape = RoundedCornerShape(corner * slid.value)
                clip = slid.value > 0f
            }
            .drawWithContent {
                drawContent()
                if (slid.value > 0f) {
                    val r = corner * slid.value
                    drawRoundRect(
                        p.frame.copy(alpha = slid.value),
                        topLeft = Offset(line / 2, line / 2),
                        size = Size(size.width - line, size.height - line),
                        cornerRadius = CornerRadius(r, r),
                        style = Stroke(line),
                    )
                }
            },
    ) {
        // Painted paper to the screen's edges, so nothing shows black
        // behind the keyboard or the navigation bar as they move. Only the
        // keyboard is kept clear here: each place keeps clear of the status
        // and navigation bars itself (Paged), but the chat, whose talk runs
        // on under them (ChatPlace).
        Column(
            Modifier.fillMaxSize().background(p.paper)
                .then(if (model.held != null || model.heldCard != null) Modifier.blur(12.dp) else Modifier)
                .windowInsetsPadding(WindowInsets.ime),
        ) {
            // The bar floats over the foot of the places as a pill (a design choice of
            // 2026-11-06): lists run on under it, while a page that ends in a
            // composer or its own foot stops above it.
            val barShown = !WindowInsets.isImeVisible
            // The room the bar takes at the foot, the gap above it included,
            // so the composer stands one gap off it (phone.tsx --bar-room).
            val room = if (barShown) FOOT_GAP + BAR_HEIGHT + FOOT_INNER else 0.dp
            val clear = Modifier.padding(bottom = room)
            // A whiteboard's and the browser's every drag is the page's
            // (draw, pan, pinch; scroll, select), so the page doesn't swipe
            // to the next item over them; the bar still does.
            val ownsDrags = over == null && shown.kind in setOf("board", "browser")
            Box(Modifier.weight(1f).fillMaxWidth().then(if (editing || ownsDrags) Modifier else Modifier.swiped(at, items.size, over ?: "") { swipe(it) })) {
            androidx.compose.runtime.CompositionLocalProvider(LocalBarRoom provides room) {
                // The items lie side by side in bar order and the page slides
                // along them; the bar holds still. The chat stays composed
                // when away, so its stream goes on.
                // It slides when another item is shown, and snaps when the shown
                // one only changes place (the bar reordered, or an item before
                // it closed): sliding then would carry the page off the screen
                // and back for nothing.
                // A tap on an item more than one place away slides as a
                // neighbour's does, not through every page between: the page
                // shown stands in beside the target for the slide (`jump`:
                // the shown page's place, and the place it stands in at),
                // and the page truly there keeps out of view meanwhile.
                val slider = remember { androidx.compose.animation.core.Animatable(at.toFloat()) }
                val sliding = remember { arrayOf(shownKey) }
                var jump by remember { mutableStateOf<Pair<Int, Int>?>(null) }
                LaunchedEffect(at, shownKey) {
                    val same = sliding[0] == shownKey
                    sliding[0] = shownKey
                    val from = kotlin.math.round(slider.value).toInt()
                    if (same) {
                        slider.snapTo(at.toFloat())
                    } else if (abs(at - from) > 1) {
                        val beside = if (at > from) at - 1 else at + 1
                        jump = from to beside
                        try {
                            slider.snapTo(beside.toFloat())
                            slider.animateTo(at.toFloat(), tween(240))
                        } finally {
                            jump = null
                        }
                    } else {
                        slider.animateTo(at.toFloat(), tween(240))
                    }
                }
                val along = slider.value
                val place = @Composable { item: PhonePane, here: Boolean ->
                    when (item.kind) {
                        "chat" -> {
                            // The tab shown: Main's chat, or a named memory's.
                            val memory = item.arg
                            if (memory == null) ChatPlace(model, draft, pins, Modifier.fillMaxSize())
                            else key(memory) { MemoryChatPlace(model, memory) }
                            // No bar: the buttons and the memories' tabs float over the
                            // talk with nothing behind them, and the talk fades out under
                            // them (ChatPlace).
                            Row(Modifier.fillMaxWidth().statusBarsPadding().padding(start = 14.dp, end = 14.dp, top = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                FloatButton(Glyph.panel, "Side panel", { panel(true) })
                                MemoryTabs(model, item, { setChat(item, it) }, Modifier.weight(1f, fill = false))
                                Spacer(Modifier.weight(1f))
                                if (memory == null) FloatButton(Glyph.find, "Search", { searching = true })
                            }
                        }
                        // Temp, Workers and the Library run up under the status bar with
                        // only a thin fade there, each padding its
                        // own first line clear of it (Kit.kt statusTop).
                        // Temp runs on under the bar and the navigation bar, as the chat does:
                        // its dock floats above them (LibriApp.kt ChatDock).
                        "temp" -> Box(Modifier.fillMaxSize()) {
                            key(item.arg) { TempPlace(model, item.arg, hold = { id -> holdTemp(item, id) }) { toChat() } }
                        }
                        "terminal" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) { key(item.arg) { TerminalScreen(model, item.arg) } }
                        "workers" -> {
                            // A thread stays open behind the other items, with its
                            // scroll and its unsent words, until it is folded.
                            val thread = model.opened
                            // A thread runs on under the bar and the navigation bar, as
                            // the chat does: its dock floats above them.
                            if (thread != null) Box(Modifier.fillMaxSize().statusBarsPadding()) { ThreadPage(model, thread, shown = here) { model.fold(); model.refreshWork() } }
                            else Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.navigationBars)) {
                                WorkPlace(model, bring = { t ->
                                    toChat()
                                    model.send("Bring back from Work: ${t.name}'s thread. Tell me what came of it and what's next; its report is in your memory.", emptyList())
                                }, openMain = { toChat() })
                            }
                        }
                        "thread" -> Box(Modifier.fillMaxSize().statusBarsPadding()) {
                            ThreadPage(model, item.arg ?: "", shown = here) { toChat() }
                        }
                        "library" -> Box(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.navigationBars)) { key(model.libraryDrawn) {
                            LibraryPlace(
                                model,
                                shown = here,
                                compose = { words ->
                                    toChat()
                                    model.draft.text = joined(model.draft.text, words)
                                },
                            ) { words ->
                                toChat()
                                model.send(words, emptyList())
                            }
                        } }
                        "pinned" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) {
                            PinnedPlace(pins, model.me?.name ?: "the assistant") { pin ->
                                toChat()
                                model.send(resumed(pin, model.me?.name ?: "the assistant"), emptyList())
                            }
                        }
                        "builtin" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) { BuiltinPlace(model, item.arg ?: "") { toChat() } }
                        // The Whiteboard tile is the host's own page of it
                        // (the site's `#board-tile=`), as Library pages are;
                        // the board the person picks there is this place's own.
                        "board" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) {
                            BoardPlace(model, item)
                        }
                        // Libri's own browser, shared with the agents: the
                        // host's page of the Browser tile (`#browser-tile`).
                        "system" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) {
                            model.systemUrl()?.let { hostLook(it, Ink) }?.let { HostWebView(it, {}, Modifier.fillMaxSize()) }
                        }
                        "browser" -> Box(Modifier.fillMaxSize().windowInsetsPadding(SYSTEM_BARS).then(clear)) {
                            model.browserUrl()?.let { hostLook(it, Ink) }?.let { HostWebView(it, {}, Modifier.fillMaxSize()) }
                        }
                    }
                }
                // A whiteboard is a web page and its editor, a second or so
                // to start: kept once made, so coming back is instant, and
                // made a little after the app opens, so the first tap finds
                // it ready, without slowing the app's own start.
                var boardsKept by remember { mutableStateOf(false) }
                LaunchedEffect(Unit) {
                    kotlinx.coroutines.delay(1500)
                    boardsKept = true
                }
                items.forEachIndexed { i, item ->
                    // The chat always, so its stream goes on.
                    key(item.key()) {
                        // The shown item always, too: for the frame before a reorder's
                        // snap it may stand a whole place off, and dropped then, the
                        // terminal would lose its shell and raise the keyboard again.
                        val j = jump
                        // Mid-jump, the shown page stands beside the target, and
                        // the one there stands far off (still composed if it must be).
                        val placed = when {
                            j == null -> i
                            i == j.first -> j.second
                            i == j.second -> Int.MIN_VALUE / 2
                            else -> i
                        }
                        Paged(placed, along, always = item.kind == "chat" || (item.kind == "board" && boardsKept) || i == at || (j != null && i == j.first), bare = true) { place(item, i == at && over == null) }
                    }
                }
                over?.let { kind ->
                    Box(Modifier.fillMaxSize().background(p.paper)) { place(PhonePane(kind, null), true) }
                }
                Box(Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 60.dp)) { Notice(model) }
            }
            // A tap anywhere off the bar puts it down; the page under takes nothing meanwhile.
            if (editing) Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { editing = false } })
            closed?.let { c ->
                Row(
                    Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = FOOT_GAP + BAR_HEIGHT + FOOT_INNER)
                        .floating(p).padding(start = 16.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("Closed ${c.name}.", style = Type.small, color = p.ink)
                    Quiet("Undo", p.ink) {
                        model.changeSpaces(
                            model.spaces.map { w ->
                                if (w.id != c.space || w.items.any { it.key() == c.item.key() }) w
                                else w.copy(items = w.items.toMutableList().also { it.add(c.at.coerceAtMost(it.size), c.item) })
                            },
                            model.spaceOn,
                        )
                        closed = null
                    }
                }
            }
            if (barShown) Box(Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = FOOT_GAP)) {
                Bar(
                    space = ws,
                    lit = if (over != null) items.indexOfFirst { it.kind == over } else at,
                    editing = editing,
                    leaving = leaving,
                    title = { itemName(model, it) },
                    marks = { item -> if (item.kind == "temp" && model.temp(tempKey(item.arg)).let { it.busy || it.unread }) model.temp(tempKey(item.arg)).let { WorkMark(if (it.unread) 1 else 0, it.busy, it.busy) } },
                    tap = { i ->
                        show(i)
                    },
                    edit = { on -> editing = on; if (on) menu = null },
                    held = { i -> menu = null; itemMenu = i },
                    move = moveItem,
                    close = closeItem,
                    spaces = { menu = if (menu == "spaces") null else "spaces" },
                    add = { menu = if (menu == "add") null else "add" },
                    swipe = { right -> goSpace(if (right) -1 else 1) },
                )
            }
            }
        }
        itemMenu?.let { i ->
            items.getOrNull(i)?.let { item ->
                ItemMenu(itemName(model, item), model.tiles?.of(item.kind)?.rows.orEmpty(), model.tiles?.barRows.orEmpty(), itemCode(i, item)) { itemMenu = null }
            }
        }
        menu?.let { m ->
            Menus(
                model, m, ws,
                close = { menu = null },
                goSpace = { id -> model.changeSpaces(model.spaces, id); shownKey = null; over = null; menu = null },
                add = { asked ->
                    // A second one is its own where the host says so (tiles.rs `fresh`).
                    val pane = another(model.tiles, asked, items)
                    if (items.none { it.key() == pane.key() }) change { it.copy(items = it.items + pane) }
                    over = null
                    shownKey = pane.key()
                    menu = null
                },
            )
        }
        if (model.spacesByAgent || model.spacesSaveError) Box(Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 64.dp)) {
            Row(
                Modifier.clip(CircleShape).background(p.lift).border(1.dp, p.hair, CircleShape).padding(start = 16.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (model.spacesSaveError) Text("Couldn’t save workspaces. Retrying…", style = Type.small, color = p.ink)
                else {
                    Text("Your workspaces were rearranged.", style = Type.small, color = p.ink)
                    Quiet("Undo", p.ink) { model.undoSpaces() }
                    Quiet("Keep") { model.keepAgentSpaces() }
                }
            }
        }
        // Open, the page in view is only a way back: a tap on it shuts the
        // panel, and nothing under it takes the touch.
        if (panelOpen) Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { panel(false) } })
    }
    }
}

/** How long a closed workspace's Undo stands where it was (tiles.tsx). */
const val UNDO_CLOSE_MS = 3_000L
/** How long Recently closed keeps a workspace (layout.rs KEPT_CLOSED). */
const val KEPT_CLOSED_DAYS = 30

/** When a workspace was closed, said briefly. */
fun closedAgo(at: Long): String {
    val m = ((System.currentTimeMillis() / 1000 - at) / 60).coerceAtLeast(0)
    return when {
        m < 1 -> "just now"
        m < 60 -> "$m min ago"
        m < 48 * 60 -> "${(m + 30) / 60} h ago"
        else -> "${(m + 720) / 1440} days ago"
    }
}

/** An item just closed from a workspace's bar, kept for its Undo. */
private data class ClosedItem(val space: String, val item: PhonePane, val at: Int, val name: String)

/** An item's name on the bar: a worker's, a built-in's, or its kind's. */
private fun itemName(model: ChatModel, item: PhonePane): String = when (item.kind) {
    "thread" -> model.threads.firstOrNull { it.id == item.arg }?.name ?: "Worker"
    "builtin" -> model.builtins.firstOrNull { it.id == item.arg }?.title ?: "Built-in"
    else -> kindName(model, item.kind)
}

private fun itemGlyph(kind: String): ImageVector = when (kind) {
    "chat" -> Glyph.chat
    "terminal" -> Glyph.terminal
    "workers" -> Glyph.work
    "thread" -> Glyph.agent
    "library" -> Glyph.library
    "board" -> Glyph.board
    "browser" -> Glyph.browser
    "system" -> Glyph.system
    "pinned" -> Glyph.pin
    "builtin" -> Glyph.calendar
    else -> Glyph.chat
}

/** The Whiteboard tile in a web view. The page tells which board is picked
 * ("board:<id>"), kept as this item's own, so each workspace keeps its tab. */
@Composable
private fun BoardPlace(model: ChatModel, item: PhonePane) {
    val p = Ink
    // The board this place opened on: a pick inside the page moves the
    // page itself, so the address stays the first, not reloading it.
    val first = remember { item.arg }
    val at = model.boardUrl(first)?.let { hostLook(it, p) } ?: return
    HostWebView(at, { message ->
        val picked = message.removePrefix("board:").takeIf { message.startsWith("board:") && it.isNotBlank() } ?: return@HostWebView
        if (picked == item.arg) return@HostWebView
        val space = model.spaces.firstOrNull { it.id == model.spaceOn } ?: return@HostWebView
        val index = space.items.indexOfFirst { it == item }.takeIf { it >= 0 } ?: return@HostWebView
        val items = space.items.toMutableList().also { it[index] = PhonePane("board", picked) }
        model.changeSpaces(model.spaces.map { if (it.id == space.id) it.copy(items = items) else it }, model.spaceOn)
    }, Modifier.fillMaxSize())
}

/** A built-in's view (Calendar) as an item of its own. */
@Composable
private fun BuiltinPlace(model: ChatModel, id: String, back: () -> Unit) {
    val p = Ink
    val b = model.builtins.firstOrNull { it.id == id }
    val at = model.builtinUrl(id)?.let { hostLook(it, p) }
    Column(Modifier.fillMaxSize()) {
        if (b == null || !b.on) {
            Text("This built-in is off. Turn it on in Settings, Connections.", style = Type.small, color = p.mid, modifier = Modifier.padding(24.dp))
        } else if (b.connection == null) {
            // No connection fills it yet: the one way forward.
            Column(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 24.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
                Text("No connection yet", style = type(TypeScale.body, 1.45f), color = p.ink)
                Text("Add a connection, such as Fastmail, and ${b.title} fills from it on its own.", style = Type.small, color = p.mid, textAlign = androidx.compose.ui.text.style.TextAlign.Center, modifier = Modifier.padding(top = 6.dp, bottom = 14.dp))
                Pressable("Add a connection", filled = true) { model.toConnections = true }
            }
        } else if (at != null) HostWebView(at, { text -> back(); model.draft.text = joined(model.draft.text, text) }, Modifier.weight(1f).fillMaxWidth())
    }
}


/** Lets a text field go once the person has left it: on moving to another
 * place or the side panel, which keep the chat's field alive out of sight, and
 * on putting the keyboard away, which leaves a field its focus. A field still
 * focused has the keyboard raised again when the window returns, so these
 * brought it up on a page with nothing to type in. Leaving the app hides the
 * keyboard too, but with the window's focus already gone: that field keeps
 * its place, and the keyboard comes back with the app. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun LetGoOfFields(place: String, panel: Boolean) {
    val focus = LocalFocusManager.current
    val window = LocalWindowInfo.current
    LaunchedEffect(place, panel) { focus.clearFocus() }
    val up = WindowInsets.isImeVisible
    var was by remember { mutableStateOf(up) }
    LaunchedEffect(up) {
        if (was && !up) {
            // Switching keyboards (FUTO's keyboard to FUTO Voice) hides one
            // before the next shows. Letting go at once leaves the new one no
            // field, so it closes; a keyboard that comes back within the wait
            // cancels this effect and the field keeps its focus.
            delay(700)
            if (window.isWindowFocused) focus.clearFocus()
        }
        was = up
    }
}

/** One place on the strip, `at` along it, drawn while any of it is in view
 * (the chat always). */
@Composable
private fun Paged(at: Int, along: Float, always: Boolean = false, bare: Boolean = false, content: @Composable BoxScope.() -> Unit) {
    if (!always && abs(at - along) >= 1f) return
    Box(
        Modifier.fillMaxSize().graphicsLayer { translationX = (at - along) * size.width }.background(Ink.paper)
            .then(if (bare) Modifier else Modifier.windowInsetsPadding(SYSTEM_BARS)),
        content = content,
    )
}

/** The status and navigation bars, which a place keeps clear of unless it
 * runs on under them, as the chat does. */
private val SYSTEM_BARS @Composable get() = WindowInsets.statusBars.union(WindowInsets.navigationBars)

/** A swipe across, for `go` with whether it went right. The drag starts only
 * if nothing inside took it first (lists take the vertical ones, code and
 * tables that scroll take their own) and only if it goes across more than
 * down (acrossNotDown): a page in a WebView reports none of its scrolling. */
private fun Modifier.swiped(vararg keys: Any, go: (Boolean) -> Unit) = pointerInput(*keys) {
    acrossNotDown { went -> if (abs(went) > 80.dp.toPx()) go(went > 0) }
}

/** A drag across, told how far it went once the finger lifts. It is taken
 * only when, by the touch slop, the finger has gone across at least twice
 * as far as down, as ViewPager decides (onInterceptTouchEvent: xDiff >
 * slop && xDiff * 0.5 > yDiff); one that passes the slop mostly downward
 * is left alone to its end. detectHorizontalDragGestures looks at the
 * across part alone, and a WebView consumes nothing Compose can see, so a
 * page's scroll that drifted a few millimetres sideways was taken from it,
 * the WebView sent a cancel, and the page stopped dead with no fling. */
private suspend fun PointerInputScope.acrossNotDown(done: (Float) -> Unit) = awaitEachGesture {
    val down = awaitFirstDown(requireUnconsumed = false)
    val slop = viewConfiguration.touchSlop
    var total = Offset.Zero
    while (true) {
        val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: return@awaitEachGesture
        if (change.changedToUpIgnoreConsumed() || change.isConsumed) return@awaitEachGesture
        total += change.positionChange()
        val across = abs(total.x)
        val downward = abs(total.y)
        if (across > slop && across * 0.5f > downward) {
            change.consume()
            break
        }
        if (downward > slop) return@awaitEachGesture
    }
    var went = total.x
    if (horizontalDrag(down.id) { went += it.positionChange().x; it.consume() }) done(went)
}

/** The bar at the foot (phone.tsx): the workspace's disc, its items, the
 * plus. Each item an icon over its name, in ink when shown and --soft
 * otherwise. A swipe along it goes to the workspace beside. No line over
 * it: lines cut across the whole screen are kept out of the app.
 *
 * It edits in place, as a phone's home
 * screen does: a long press lifts every item but the disc, which is pinned;
 * dragged sideways an item takes a new place, dropped on the plus (an ✕
 * while editing) it leaves the bar, and a tap off the bar puts it down. Any
 * time, an item flicked up leaves the bar, as a card flicked away in the
 * app switcher, with an Undo (Places). */
@Composable
private fun Bar(
    space: PhoneSpace,
    lit: Int,
    editing: Boolean,
    leaving: String?,
    title: (PhonePane) -> String,
    marks: @Composable BoxScope.(PhonePane) -> Unit,
    tap: (Int) -> Unit,
    edit: (Boolean) -> Unit,
    /** A hold on an item: its menu. */
    held: (Int) -> Unit,
    move: (Int, Int) -> Unit,
    close: (Int) -> Unit,
    spaces: () -> Unit,
    add: () -> Unit,
    swipe: (Boolean) -> Unit,
) {
    val p = Ink
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    // The item carried: where it started, how far it has gone, and whether
    // the finger is over the ✕.
    var carried by remember { mutableStateOf<Pair<Int, Float>?>(null) }
    var overX by remember { mutableStateOf(false) }
    val plusAt = remember { arrayOf(androidx.compose.ui.geometry.Rect.Zero) }
    val editingNow = androidx.compose.runtime.rememberUpdatedState(editing)
    Row(
        Modifier.padding(horizontal = FOOT_GAP).pointerInput(space.id, editing) {
            if (!editing) acrossNotDown { went -> if (abs(went) > 48.dp.toPx()) swipe(went > 0) }
        },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(FOOT_INNER),
    ) {
        // A disc, so the bar keeps its room: the name's first letter. Pinned:
        // editing leaves it alone. No edge on it or the pill: told apart by
        // fill and shadow alone, as Claude Duet's bar (2026-11-13).
        Box(
            Modifier.size(BAR_HEIGHT).floating(p, edge = false)
                .clickable(onClickLabel = "Switch workspace", onClick = spaces)
                .semantics { contentDescription = "Workspace: ${space.name}. Switch workspace" },
            contentAlignment = Alignment.Center,
        ) {
            Text(space.name.trim().take(1).uppercase().ifEmpty { "·" }, style = type(TypeScale.control, 1.3f, FontWeight.Medium), color = p.ink, maxLines = 1)
        }
        // Three items fill the room beside the disc; fewer take their
        // thirds of it, more share it down to 56dp each, then scroll. The
        // plus rides at the pill's end.
        BoxWithConstraints(Modifier.weight(1f), contentAlignment = Alignment.Center) {
            val n = space.items.size
            val room = maxWidth - 8.dp - PLUS_WIDTH
            val each = maxOf(56.dp, room / maxOf(3, n))
            val eachPx = with(LocalDensity.current) { each.toPx() }
            val scrolls = each * n > room
            Row(
                Modifier.height(BAR_HEIGHT).then(if (scrolls) Modifier.fillMaxWidth() else Modifier).floating(p, edge = false).padding(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    Modifier.then(if (scrolls) Modifier.weight(1f).horizontalScroll(rememberScrollState(), enabled = carried == null) else Modifier).fillMaxHeight(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    space.items.forEachIndexed { i, item ->
                        key(item.key()) {
                            val c = carried
                            val to = c?.let { (from, dx) -> (from + kotlin.math.round(dx / eachPx).toInt()).coerceIn(0, n - 1) }
                            // While one is carried the others step aside to its landing place.
                            val shift = when {
                                c == null || c.first == i || to == null -> 0f
                                i > c.first && i <= to -> -eachPx
                                i < c.first && i >= to -> eachPx
                                else -> 0f
                            }
                            val slide by animateFloatAsState(shift, tween(200), label = "tab-shift")
                            val gone = leaving == item.key()
                            val width by androidx.compose.animation.core.animateDpAsState(if (gone) 0.dp else each, tween(180), label = "tab-width")
                            val fade by animateFloatAsState(if (gone) 0f else 1f, tween(180), label = "tab-fade")
                            val rise by animateFloatAsState(if (gone) -24f else if (editing) -2f else 0f, tween(180), label = "tab-rise")
                            val coords = remember { arrayOf<androidx.compose.ui.layout.LayoutCoordinates?>(null) }
                            Tab(
                                title(item), i == lit, width, editing,
                                Modifier.onGloballyPositioned { coords[0] = it }
                                    .then(if (c?.first == i) Modifier.zIndex(1f) else Modifier)
                                    .graphicsLayer {
                                        val carrying = c?.first == i
                                        translationX = if (carrying) c!!.second else slide
                                        translationY = rise.dp.toPx() + if (carrying) (-2).dp.toPx() else 0f
                                        val k = if (carrying) 1.06f else 1f
                                        scaleX = k
                                        scaleY = k
                                        alpha = fade * if (carrying && overX) 0.6f else 1f
                                    }
                                    .pointerInput(i, n, eachPx) {
                                        itemGesture(
                                            editing = { editingNow.value },
                                            hold = { haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress); held(i) },
                                            tap = { if (!editingNow.value) tap(i) },
                                            flick = { close(i) },
                                            carry = { dx, at ->
                                                carried = i to dx
                                                overX = onPlus(coords[0], plusAt[0], dx, at)
                                            },
                                            drop = { dx, at ->
                                                val onX = onPlus(coords[0], plusAt[0], dx, at)
                                                carried = null
                                                overX = false
                                                if (onX) close(i) else move(i, (i + kotlin.math.round(dx / eachPx).toInt()).coerceIn(0, n - 1))
                                            },
                                            cancel = { carried = null; overX = false },
                                        )
                                    }
                                    .semantics {
                                        onClick { if (editing) edit(false) else tap(i); true }
                                        customActions = if (editing) listOf(
                                            androidx.compose.ui.semantics.CustomAccessibilityAction("Move left") { move(i, i - 1); true },
                                            androidx.compose.ui.semantics.CustomAccessibilityAction("Move right") { move(i, i + 1); true },
                                            androidx.compose.ui.semantics.CustomAccessibilityAction("Take off the bar") { close(i); true },
                                        ) else listOf(
                                            androidx.compose.ui.semantics.CustomAccessibilityAction("Close") { close(i); true },
                                            androidx.compose.ui.semantics.CustomAccessibilityAction("Edit the bar") { edit(true); true },
                                        )
                                    },
                                mark = { marks(item) },
                            ) {
                                if (item.kind == "temp") TempGlyph(22.dp) else Glyph(itemGlyph(item.kind), 22.dp)
                            }
                        }
                    }
                }
                // While editing, the plus is the ✕ an item is dropped on to
                // leave the bar; a tap on it puts the bar down.
                val label = if (editing) "Done editing the bar" else "Add to ${space.name}"
                Box(
                    Modifier.width(PLUS_WIDTH).fillMaxHeight().onGloballyPositioned { plusAt[0] = it.boundsInRoot() }
                        .clip(CircleShape).then(if (overX) Modifier.background(p.accent.copy(alpha = 0.15f)) else Modifier)
                        .clickable(onClickLabel = label) { if (editing) edit(false) else add() }
                        .semantics { contentDescription = label },
                    contentAlignment = Alignment.Center,
                ) { Glyph(if (editing) Glyph.close else Glyph.plus, 22.dp, if (overX) p.accent else if (editing) p.ink else p.mid) }
            }
        }
    }
}

/** Whether a carried item's finger is over the plus (the ✕ while editing).
 * The item's coordinates are taken before its graphicsLayer, while the
 * finger's position comes from inside it, so the carry offset dx is added
 * back to land in the bar's real space. */
private fun onPlus(item: androidx.compose.ui.layout.LayoutCoordinates?, plus: androidx.compose.ui.geometry.Rect, dx: Float, at: androidx.compose.ui.geometry.Offset): Boolean =
    item != null && item.isAttached && plus.inflate(12f).contains(item.localToRoot(at.copy(x = at.x + dx)))

/** One finger on a bar item: a tap shows it, a quick flick up closes it, a
 * hold starts editing, and while editing a drag sideways carries it. Nothing
 * is taken from the bar's own swipe or scroll until the item is held. */
private suspend fun PointerInputScope.itemGesture(
    editing: () -> Boolean,
    hold: () -> Unit,
    tap: () -> Unit,
    flick: () -> Unit,
    carry: (Float, Offset) -> Unit,
    drop: (Float, Offset) -> Unit,
    cancel: () -> Unit,
) = awaitEachGesture {
    val down = awaitFirstDown(requireUnconsumed = false)
    val slop = viewConfiguration.touchSlop
    var held = editing()
    var moved = false
    var carrying = false
    var total = Offset.Zero
    while (true) {
        val waited = down.uptimeMillis + viewConfiguration.longPressTimeoutMillis
        val event = if (held || moved) awaitPointerEvent() else withTimeoutOrNull(waited - currentEvent.changes.first().uptimeMillis) { awaitPointerEvent() }
        if (event == null) {
            held = true
            hold()
            continue
        }
        val change = event.changes.firstOrNull { it.id == down.id } ?: return@awaitEachGesture cancel()
        if (change.changedToUpIgnoreConsumed()) {
            if (carrying) {
                change.consume()
                drop(total.x, change.position)
            } else if (!held && !editing()) {
                val t = change.uptimeMillis - down.uptimeMillis
                // Quick and mostly upward: a flick, not a slow drag nor a sideways swipe.
                if (total.y < -36.dp.toPx() && abs(total.y) > abs(total.x) * 1.5f && t < 400) flick()
                else if (!moved) tap()
            }
            return@awaitEachGesture
        }
        if (!held && change.isConsumed) return@awaitEachGesture
        total += change.positionChange()
        if (!moved && total.getDistance() > slop) moved = true
        if (held) {
            change.consume()
            if (!carrying && abs(total.x) > slop) carrying = true
            if (carrying) carry(total.x, change.position)
        }
    }
}

/** The plus at the pill's end: the web's 3rem. */
private val PLUS_WIDTH = 48.dp

/** The composer's height at rest: one line and its controls row, 81dp at
 * the default font scale on the emulator (the page's is 88px). */
private val COMPOSER_REST = 81.dp

/** The floating bar's height, two thirds of the composer's at rest: the two
 * floating pills at the foot in proportion, as Claude Duet's (photo,
 * 2026-11-13; at the composer's full height it stood too heavy, 11-13).
 * phone.tsx BAR. */
val BAR_HEIGHT = COMPOSER_REST * 2 / 3

/** The gap at the foot outside its pills: from the screen's sides, and
 * from the bar to the gesture area. phone.tsx GAP. */
val FOOT_GAP = 14.dp

/** The gap between the pills at the foot: composer to bar, disc to tabs.
 * Between the send button's inset (5dp, 2026-11-13), which read as
 * squeezed, and the outer gap it once shared (14dp): a little more room,
 * the pills still one group (2026-11-14). phone.tsx INNER. */
private val FOOT_INNER = 10.dp

@Composable
private fun Tab(label: String, on: Boolean, width: androidx.compose.ui.unit.Dp, editing: Boolean, modifier: Modifier, mark: @Composable BoxScope.() -> Unit = {}, icon: @Composable () -> Unit) {
    val p = Ink
    val tint = if (on) p.ink else if (editing) p.mid else p.soft
    Column(
        modifier.width(width).fillMaxHeight()
            // Lifted while editing, each its own floating chip, as the web's.
            .then(if (editing) Modifier.padding(horizontal = 2.dp).floating(p) else Modifier.clip(CircleShape))
            .then(if (on && !editing) Modifier.background(p.hover) else Modifier)
            .semantics(mergeDescendants = true) { role = androidx.compose.ui.semantics.Role.Tab; selected = on },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box {
            androidx.compose.runtime.CompositionLocalProvider(androidx.compose.material3.LocalContentColor provides tint) { icon() }
            mark()
        }
        // At five items a tab is 56dp, too narrow for "Terminal" at caption
        // size: the label steps down to fit rather than spill past the tab.
        val style = type(TypeScale.caption, 1.3f, if (on) FontWeight.Medium else FontWeight.Normal)
        Text(
            label, Modifier.padding(horizontal = 4.dp), style = style, color = tint, maxLines = 1,
            autoSize = androidx.compose.foundation.text.TextAutoSize.StepBased(minFontSize = 9.sp, maxFontSize = style.fontSize),
        )
    }
}

/** What opens over the bar: the workspaces (switch, rename, close, new) or
 * the plus's menu. */
@Composable
private fun Menus(
    model: ChatModel,
    menu: String,
    ws: PhoneSpace,
    close: () -> Unit,
    goSpace: (String) -> Unit,
    add: (PhonePane) -> Unit,
) {
    val p = Ink
    val context = LocalContext.current
    val pinsOn = remember { Pins(context) }.on.value
    BackHandler(onBack = close)
    Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { close() } }) {
        val card = Modifier.navigationBarsPadding().padding(bottom = FOOT_GAP + BAR_HEIGHT + FOOT_INNER)
            .shadow(8.dp, RoundedCornerShape(Radii.xl), ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
            .clip(RoundedCornerShape(Radii.xl)).background(p.lift).border(1.dp, p.hair, RoundedCornerShape(Radii.xl))
            .pointerInput(Unit) { detectTapGestures { } }
            .padding(4.dp)
        val row = { label: String, enabled: Boolean, note: String?, glyph: (@Composable () -> Unit)?, go: () -> Unit ->
            @Composable {
                Row(
                    Modifier.fillMaxWidth().height(48.dp).clip(RoundedCornerShape(Radii.lg)).clickable(enabled = enabled, onClick = go).padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (glyph != null) Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) { glyph() }
                    Text(label, style = Type.small, color = if (enabled) p.ink else p.soft, modifier = Modifier.weight(1f), maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    if (note != null) Text(note, style = Type.note, color = p.soft)
                }
            }
        }
        when {
            menu == "spaces" -> Column(Modifier.align(Alignment.BottomCenter).padding(horizontal = 8.dp).fillMaxWidth().then(card).verticalScroll(rememberScrollState())) {
                Text("Workspaces", style = Type.note, color = p.soft, modifier = Modifier.padding(start = 12.dp, top = 8.dp, bottom = 4.dp))
                var naming by remember { mutableStateOf<String?>(null) }
                var made by remember { mutableStateOf<String?>(null) }
                var open by remember { mutableStateOf<String?>(null) }
                val rename = { id: String, name: String ->
                    if (name.isNotBlank()) model.changeSpaces(model.spaces.map { if (it.id == id) it.copy(name = name.trim()) else it }, model.spaceOn)
                    naming = null
                    if (made == id) close()
                    made = null
                }
                // A workspace just closed: for a moment an Undo stands where it was.
                var undo by remember { mutableStateOf<Pair<PhoneClosed, Int>?>(null) }
                LaunchedEffect(undo) { if (undo != null) { delay(UNDO_CLOSE_MS); undo = null } }
                val undoRow = @Composable {
                    undo?.let { (c, at) ->
                        row("Closed ${c.name}", true, "Undo", null) { model.reopenSpace(c.id, at); undo = null }()
                    }
                }
                model.spaces.forEachIndexed { i, w ->
                    if (undo?.second == i) undoRow()
                    Row(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).then(if (w.id == ws.id) Modifier.background(p.paper) else Modifier),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (naming == w.id) {
                            var text by remember { mutableStateOf(androidx.compose.ui.text.input.TextFieldValue(w.name, androidx.compose.ui.text.TextRange(0, w.name.length))) }
                            val focus = remember { androidx.compose.ui.focus.FocusRequester() }
                            LaunchedEffect(Unit) { focus.requestFocus() }
                            androidx.compose.foundation.text.BasicTextField(
                                text, { text = it },
                                singleLine = true,
                                textStyle = Type.small.copy(color = p.ink),
                                cursorBrush = androidx.compose.ui.graphics.SolidColor(p.ink),
                                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(imeAction = androidx.compose.ui.text.input.ImeAction.Done),
                                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { rename(w.id, text.text) }),
                                modifier = Modifier.weight(1f).padding(4.dp).clip(RoundedCornerShape(Radii.lg)).background(p.paper).border(2.dp, p.frame, RoundedCornerShape(Radii.lg)).padding(horizontal = 12.dp, vertical = 10.dp).focusRequester(focus)
                                    .semantics { contentDescription = "Workspace name" },
                            )
                        } else {
                            Column(Modifier.weight(1f).clickable { goSpace(w.id) }.padding(horizontal = 12.dp, vertical = 8.dp)) {
                                Text(w.name, style = type(TypeScale.control, 1.4f, if (w.id == ws.id) FontWeight.Medium else FontWeight.Normal), color = if (w.id == ws.id) p.ink else p.mid, maxLines = 1)
                                Text(w.items.joinToString(", ") { itemName(model, it) }, style = Type.note, color = p.soft, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                            }
                        }
                        Round(onClick = { open = if (open == w.id) null else w.id }, size = 40.dp) { Glyph(Glyph.more, 18.dp, p.soft, label = "More for ${w.name}") }
                    }
                    if (open == w.id) Column(Modifier.padding(start = 12.dp)) {
                        row("Rename", true, null, null) { open = null; naming = w.id }()
                        row("Close workspace", true, null, null) {
                            open = null
                            val at = model.closeSpace(w.id)
                            model.spacesClosed.firstOrNull { it.id == w.id }?.let { undo = it to at }
                        }()
                    }
                }
                undo?.let { (_, at) -> if (at >= model.spaces.size) undoRow() }
                row("New workspace", true, null, { Glyph(Glyph.plus, 18.dp, p.mid) }) {
                    val w = PhoneSpace(freshSpaceId(), unnamedSpace(model.spaces.map { it.name }), listOf(PhonePane("chat", null)))
                    // Shown at once, its name offered to edit; named, the list goes.
                    model.changeSpaces(model.spaces + w, w.id)
                    naming = w.id
                    made = w.id
                }()
                if (model.spacesClosed.isNotEmpty()) {
                    Box(Modifier.fillMaxWidth().padding(top = 4.dp).height(1.dp).background(p.hair))
                    Text("Recently closed", style = Type.note, color = p.soft, modifier = Modifier.padding(start = 12.dp, top = 8.dp, bottom = 4.dp))
                    model.spacesClosed.forEach { c ->
                        Row(Modifier.fillMaxWidth().height(48.dp).semantics(mergeDescendants = false) { contentDescription = "Closed ${c.name}" }, verticalAlignment = Alignment.CenterVertically) {
                            Text(c.name, style = Type.small, color = p.ink, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.padding(start = 12.dp).weight(1f, fill = false))
                            Text(closedAgo(c.closedAt.toLong()), style = Type.note, color = p.soft, maxLines = 1, modifier = Modifier.padding(start = 8.dp).weight(1f))
                            Box(Modifier.height(40.dp).clip(CircleShape).clickable { model.reopenSpace(c.id); close() }.semantics { contentDescription = "Reopen ${c.name}" }.padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                                Text("Reopen", style = Type.note, color = p.ink)
                            }
                            Round(onClick = { model.forgetSpace(c.id) }, size = 40.dp) { Glyph(Glyph.close, 16.dp, p.soft, label = "Delete ${c.name}") }
                        }
                    }
                }
                Text("Closing keeps every chat and worker, and the workspace under Recently closed for $KEPT_CLOSED_DAYS days. Swipe along the bar to switch.", style = Type.note, color = p.soft, modifier = Modifier.padding(start = 12.dp, end = 12.dp, top = 2.dp, bottom = 8.dp))
            }
            menu == "add" -> Column(Modifier.align(Alignment.BottomEnd).padding(end = 8.dp).width(260.dp).then(card).verticalScroll(rememberScrollState())) {
                Text("Add to ${ws.name}", style = Type.note, color = p.soft, modifier = Modifier.padding(start = 12.dp, top = 8.dp, bottom = 4.dp))
                // What the host says the plus offers (tiles.rs `offered`).
                val offered = model.tiles?.offered().orEmpty().map { PhonePane(it, null) } +
                    (if (pinsOn) listOf(PhonePane("pinned", null)) else emptyList()) +
                    model.builtins.filter { it.on }.map { PhonePane("builtin", it.id) }
                offered.forEach { pane ->
                    row(if (pane.kind == "temp") "Temp chat" else itemName(model, pane), true, null, {
                        if (pane.kind == "temp") TempGlyph(18.dp, p.mid) else Glyph(itemGlyph(pane.kind), 18.dp, p.mid)
                    }) { add(pane) }()
                }
                // Every worker, laid over the item shown: no item of the bar.
                row("Workers", true, null, { Glyph(Glyph.work, 18.dp, p.mid) }) { close(); model.allWorkersAsked++ }()
            }
        }
    }
}

/** On the Work tab, as the page's: one dot, red while a thread waits to be
 * read, else green, pulsing while any runs and steady if all are paused on
 * another. Red outranks green. Temp wears the same: green while its reply
 * comes, red once it has come unread. */
@Composable
private fun BoxScope.WorkMark(waiting: Int, working: Boolean, running: Boolean) {
    val p = Ink
    val dot = Modifier.align(Alignment.TopEnd).offset(x = 4.dp, y = (-2).dp).size(8.dp)
    if (waiting > 0) {
        Box(dot.background(p.accent, CircleShape))
    } else if (running) {
        val pulse by rememberInfiniteTransition(label = "work").animateFloat(
            initialValue = 1f,
            targetValue = 0.5f,
            animationSpec = infiniteRepeatable(tween(1000, easing = LinearEasing), RepeatMode.Reverse),
            label = "work-dot",
        )
        Box(dot.alpha(pulse).background(Active, CircleShape))
    } else if (working) {
        Box(dot.background(Active, CircleShape))
    }
}

/** Temp's mark: the chat bubble drawn dashed, as the page draws it, for a
 * talk nothing keeps. ImageVector has no dashes, so it is drawn here. */
@Composable
fun TempGlyph(size: Dp, tint: Color = androidx.compose.material3.LocalContentColor.current) {
    val path = remember { PathParser().parsePathString("M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z").toPath() }
    Canvas(Modifier.size(size)) {
        val k = this.size.width / 24f
        scale(k, k, Offset.Zero) {
            drawPath(path, tint, style = Stroke(width = 2f, cap = StrokeCap.Round, join = StrokeJoin.Round, pathEffect = PathEffect.dashPathEffect(floatArrayOf(3f, 2.6f))))
        }
    }
}

/** Pinned messages, newest pin first: each comes back into the chat, or
 * is let go. */
@Composable
private fun PinnedPlace(pins: Pins, name: String, resume: (Pin) -> Unit) {
    val p = Ink
    // No head, as on Chat, Tasks and the Library.
    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(horizontal = 14.dp, vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            items(pins.list, key = { it.id + it.at }) { pin ->
                Column(Modifier.fillMaxWidth().card(p, radius = Radii.listed, framed = false).border(1.dp, p.hair, RoundedCornerShape(Radii.listed)).padding(14.dp)) {
                    Head(if (pin.mine) "You" else name)
                    var open by remember { mutableStateOf(false) }
                    Text(pin.text, style = Type.small, color = p.ink, maxLines = if (open) Int.MAX_VALUE else 6, modifier = Modifier.padding(top = 4.dp).clickable { open = !open })
                    Row(Modifier.fillMaxWidth().padding(top = 6.dp), horizontalArrangement = Arrangement.End) {
                        Quiet("Unpin") { pins.drop(pin) }
                        Quiet("Bring to chat") { resume(pin) }
                    }
                }
            }
        }
    }
}

/** A small text button in --soft, as the page's card actions. */
@Composable
fun Quiet(label: String, color: Color = Color.Unspecified, onClick: () -> Unit) {
    Text(
        label,
        style = type(TypeScale.caption, 1.3f),
        color = if (color == Color.Unspecified) Ink.soft else color,
        modifier = Modifier.clip(RoundedCornerShape(Radii.lg)).clickable(onClick = onClick).padding(horizontal = 8.dp, vertical = 6.dp),
    )
}

/** A chip of Temp's Promote row: a 36dp [FloatPill], as the page's. */
@Composable
private fun Outlined(label: String, glyph: ImageVector? = null, onClick: () -> Unit) {
    val p = Ink
    FloatPill(36.dp, onClick) {
        if (glyph != null) Glyph(glyph, 14.dp, p.mid)
        Text(label, style = type(TypeScale.caption, 1.4f), color = p.mid)
    }
}

/** The scratchpad (App.tsx Temp): a place like Chat and Work, with a
 * dashed composer, its own model, thinking and
 * access, and a Memory switch for what its next turn has. A full chat the host
 * keeps, words, calls and results, and remembered by nothing unless promoted. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TempPlace(model: ChatModel, arg: String?, hold: (String) -> Unit, left: () -> Unit) {
    val p = Ink
    // A tab opened beside a Temp (arg `new`, tiles.rs) makes a Temp of its
    // own and holds it by its id from then on; one with no arg holds the
    // host's current Temp, as before there were several.
    LaunchedEffect(arg) { if (arg == "new") model.newTemp()?.let(hold) }
    val temp = model.temp(tempKey(arg))
    // Its Temp closed elsewhere (another screen, the item's Undo after it
    // went): a fresh one takes its place.
    LaunchedEffect(temp.tempId, temp.view.steps) { if (arg != null && arg != "new" && temp.view.steps != null && temp.tempId == null) hold("new") }
    val view = temp.view
    val draft = view.draft
    val list = view.list
    val follow = view.follow
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                temp.read()
                delay(ChatModel.TEMP_READ_MS)
            }
        }
    }
    val name = model.me?.name ?: model.info?.assistant ?: "It"
    val all = view.steps.orEmpty()
    val started = all.isNotEmpty() || view.telling.isNotEmpty()
    val working = temp.busy
    // The mail drafts Temp made wait here for the person's word.
    val chat by model.chat.collectAsStateWithLifecycle()
    val cards = chat?.cards.orEmpty().filter { it.scratchpad }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val room = maxHeight
        val top = statusTop
        // The composer and what Temp offers float over the talk's foot, as
        // the chat's dock (LibriApp.kt ChatDock).
        val dock = remember { DockRoom() }
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize()) {
                val foot = buildList<Pair<String, @Composable () -> Unit>> {
                    cards.forEach { c -> add("c/${c.id}" to { Letter(c) { yes -> model.decide(c.id, yes) } }) }
                    temp.asks.forEach { a -> add("a/${a.id}" to { AskCard(a) { yes, answers, always, failed -> model.reply(a.id, yes, answers, temp = true, always = always, failed = failed) } }) }
                }
                ChatTalk(
                    model, logMessages(all, view.telling, working, temp.messages(), model.chatDetail), list, follow, working, draft,
                    // Hidden under the status bar, fading in just below it.
                    fadeHide = top,
                    fadeTop = top + CHAT_TOP_FADE,
                    // 24dp above the dock: 12 here and the 12 spacing the held blank stands apart by.
                    // At the head, room under the status bar and the floating controls.
                    padding = PaddingValues(start = 16.dp, end = 16.dp, top = top + CHROME_TOP, bottom = dock.under + 12.dp),
                    spacing = 12.dp,
                    under = dock.under,
                    floor = dock.floor,
                    waiting = temp.asks.isNotEmpty(),
                    again = { text, _ -> temp.tell(text, emptyList()) },
                    foot = foot,
                    empty = if (started) null else ({
                        Text(
                            if (model.tempMemory) "$name reads the memory and has your tools here, and none of this goes into memory unless you promote it."
                            else "$name has only the web here and no memory. None of this is kept unless you promote it.",
                            style = type(TypeScale.label, 1.6f),
                            color = p.soft,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth().padding(horizontal = 40.dp, vertical = 40.dp),
                        )
                    }),
                )
                // No head: the bar names the place and leads
                // back to the chat, and Memory and Clear float at the top right
                // over the talk, as the chat's buttons do (App.tsx Temp).
                Row(
                    Modifier.align(Alignment.TopEnd).padding(top = top + 10.dp, end = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FloatPill(onClick = { model.tempMemory(!model.tempMemory) }, modifier = Modifier.semantics { contentDescription = if (model.tempMemory) "Memory on" else "Memory off" }) {
                        Text("Memory", style = type(TypeScale.label, 1.3f), color = p.mid)
                        Box(Modifier.scale(0.7f)) { PaperSwitch(model.tempMemory, model::tempMemory) }
                    }
                    if (started) FloatPill(onClick = { temp.clear(); if (arg != null && arg != "new") hold("new") }) {
                        Glyph(Glyph.eraser, 16.dp, p.mid)
                        Text("Clear", style = type(TypeScale.label, 1.3f), color = p.mid)
                    }
                }
            }
            ChatDock(dock) {
            if (started) {
                Row(Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 6.dp, bottom = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                    Outlined("Promote to chat") { temp.promote(false); left() }
                    Outlined("Create task", Glyph.work) { temp.promote(true); left() }
                }
            }
            Box(Modifier.padding(start = 14.dp, end = 14.dp, top = 4.dp, bottom = 8.dp)) {
                ComposerBox(
                    model, draft, "Message ${model.app}", room * 0.3f, dashed = true,
                    account = model.account(model.models?.tempHarness, model.models?.tempModel),
                    harness = model.models?.tempHarness,
                    pickers = { at -> Pickers(model, at, temp = true) },
                ) { empty ->
                    if (working && empty) {
                        StopButton(enabled = !temp.stopping) { temp.stop() }
                    } else {
                        SendButton(enabled = !empty) {
                            temp.tell(draft.text.trim(), draft.files.toList())
                            draft.text = ""
                            draft.files.clear()
                            follow.pinned = true
                        }
                    }
                }
            }
            }
        }
    }
}

/** A named memory's own chat, a chat item's tab (MemoryTabs.kt; App.tsx's
 * memory tile): a full chat as Temp's, on that memory alone, its name on
 * its tab above. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MemoryChatPlace(model: ChatModel, id: String) {
    val p = Ink
    LaunchedEffect(id) { model.openMemory(id) }
    val view = model.memoryView(id)
    val draft = view.draft
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(id, owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.readMemoryChat(id)
                delay(ChatModel.TEMP_READ_MS)
            }
        }
    }
    val memory = model.memories.firstOrNull { it.id == id }
    val name = memory?.name ?: "Memory"
    val all = view.steps.orEmpty()
    val working = model.memoryBusy
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val room = maxHeight
        val top = statusTop
        val dock = remember { DockRoom() }
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize()) {
                val foot = model.memoryAsks.map { a ->
                    "a/${a.id}" to @Composable { AskCard(a) { yes, answers, always, failed -> model.replyMemory(id, a.id, yes, answers, always, failed) } }
                }
                ChatTalk(
                    model, logMessages(all, view.telling, working, model.memoryMessages(), model.chatDetail), view.list, view.follow, working, draft,
                    fadeHide = top,
                    fadeTop = top + CHAT_TOP_FADE,
                    padding = PaddingValues(start = 16.dp, end = 16.dp, top = top + CHROME_TOP, bottom = dock.under + 12.dp),
                    spacing = 12.dp,
                    under = dock.under,
                    floor = dock.floor,
                    waiting = model.memoryAsks.isNotEmpty(),
                    again = { text, _ -> model.tellMemory(id, text, emptyList()) },
                    foot = foot,
                    empty = if (all.isNotEmpty() || view.telling.isNotEmpty()) null else ({
                        Text(
                            if (memory == null && model.memories.isNotEmpty()) "This memory is gone." else "The $name memory's chat. It reads and remembers in $name alone.",
                            style = type(TypeScale.label, 1.6f),
                            color = p.soft,
                            textAlign = TextAlign.Center,
                            modifier = Modifier.fillMaxWidth().padding(horizontal = 40.dp, vertical = 40.dp),
                        )
                    }),
                )
            }
            ChatDock(dock) {
                Box(Modifier.padding(start = 14.dp, end = 14.dp, top = 4.dp, bottom = 8.dp)) {
                    // No pickers: the room they would take holds the send button at the right.
                    ComposerBox(model, draft, messageTo(memory?.name), room * 0.3f, pickers = { at -> Spacer(at) }) { empty ->
                        if (working && empty) {
                            StopButton(enabled = !model.memoryStopping) { model.stopMemory(id) }
                        } else {
                            SendButton(enabled = !empty) {
                                model.tellMemory(id, draft.text.trim(), draft.files.toList())
                                draft.text = ""
                                draft.files.clear()
                                view.follow.pinned = true
                            }
                        }
                    }
                }
            }
        }
    }
}

fun Modifier.dashedEdge(color: Color, radius: Dp) = this.drawBehind {
    drawRoundRect(
        color, cornerRadius = androidx.compose.ui.geometry.CornerRadius(radius.toPx()),
        style = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx()))),
    )
}

/** The side panel (App.tsx Panel), as Tern's drawer:
 * the host's state at the top, then Hosts and Settings, and at
 * the foot the theme and the version. Three levels slide over one another:
 * the host, the settings' list, one category. */
@Composable
private fun SidePanel(model: ChatModel, pins: Pins, open: Boolean, close: () -> Unit) {
    val p = Ink
    var page by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    val version = remember { context.packageManager.getPackageInfo(context.packageName, 0).versionName.orEmpty() }
    LaunchedEffect(open) {
        if (!open) page = null
        else {
            model.refreshBuiltins()
            model.probeHosts()
        }
    }
    LaunchedEffect(model.toMemory) { if (model.toMemory) page = "Memory" }
    LaunchedEffect(model.toConnections) { if (model.toConnections) { page = "Connections"; model.toConnections = false } }
    val up = { page = if (page == null || page == "Settings") null else "Settings" }
    if (page != null) BackHandler(onBack = up)
    Column(Modifier.fillMaxHeight().statusBarsPadding().navigationBarsPadding()) {
        // The mark and the name, as Duet's panel heads itself; spacing, not
        // a rule, parts it from what follows. No close button: the page in
        // view beside the panel, Back and a swipe each put it away.
        if (page != null) Row(Modifier.fillMaxWidth().height(72.dp).padding(start = 8.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Round(onClick = up, size = 36.dp) { Glyph(Glyph.chevronLeft, 18.dp, p.mid, label = "Back") }
            Text(page ?: "", style = Type.title, color = p.ink, modifier = Modifier.weight(1f).padding(start = 12.dp))
        } else HostStatus(model)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(vertical = 8.dp)) {
            when (page) {
                // The places the chat's own can show: the chat, and each
                // built-in with a view while it is on (a design choice of 2026-11-06).
                // The host and the memory moved into Settings.
                null -> {
                    PanelRow(Glyph.hosts, "Hosts", model.hosts.size.takeIf { it > 0 }?.toString()) { page = "Hosts" }
                    PanelRow(Glyph.gear, "Settings", null) { page = "Settings" }
                }
                "Settings" -> SettingsList(model, pins) { page = it }
                "Appearance" -> Appearance(model)
                "Chat" -> ChatSettings(model, pins)
                "Notifications" -> Notifications(model)
                "Memory" -> Memory(model)
                "Models" -> ModelsPage(model)
                "Usage" -> UsagePage(model)
                "Workers" -> WorkersPage(model)
                "Guide" -> GuidePage(model, host = false)
                "HostGuide" -> GuidePage(model, host = true)
                "Connections" -> ConnectionsPage(model)
                "Voice" -> VoicePage(model)
                "Hosts" -> Hosts(model)
            }
        }
        // The foot is the panel's own: inside Settings it goes, as on the
        // page. The theme flips at a tap; the version is this build's, which
        // Obtainium updates.
        if (page == null) {
            Box(Modifier.fillMaxWidth().height(1.dp).background(p.hair))
            Row(Modifier.fillMaxWidth().height(60.dp).padding(start = 12.dp, end = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                val dark = p.dark
                Row(
                    Modifier.clip(RoundedCornerShape(Radii.pill)).clickable(onClickLabel = if (dark) "Switch to light" else "Switch to dark") { model.pickTheme(if (dark) "light" else "dark") }
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Glyph(if (dark) Glyph.moon else Glyph.sun, 20.dp, p.mid)
                    Text(if (dark) "Dark" else "Light", style = type(TypeScale.body, 1.4f), color = p.ink)
                }
                Spacer(Modifier.weight(1f))
                Text("Libri $version", style = Type.code, color = p.soft)
            }
        }
    }
}

/** The host's state at the panel's head, as Tern's drawer heads itself:
 * its initial, its name, where it is and whether it answers, and how its
 * workers stand. */
@Composable
private fun HostStatus(model: ChatModel) {
    val p = Ink
    val name = model.info?.name ?: "Libri"
    val seen = model.hostsSeen
    val address = model.address.orEmpty()
    val state = when {
        address !in seen -> "Connecting"
        seen[address] == null -> "Unreachable"
        else -> "Connected"
    }
    Column(Modifier.fillMaxWidth().padding(start = 24.dp, end = 16.dp, top = 24.dp, bottom = 20.dp)) {
        Box(Modifier.size(48.dp).background(p.ink, CircleShape), contentAlignment = Alignment.Center) {
            Text(name.take(1).uppercase(), style = Type.title, color = p.paper)
        }
        Text(name, style = type(TypeScale.title, 1.25f, FontWeight.Medium), color = p.ink, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.padding(top = 12.dp))
        Row(Modifier.padding(top = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(address.removePrefix("https://").removePrefix("http://"), style = type(TypeScale.body, 1.4f), color = p.mid, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
            Box(Modifier.size(8.dp).background(if (state == "Connected") Active else if (state == "Unreachable") p.accent else p.soft, CircleShape))
            Text(state, style = type(TypeScale.body, 1.4f), color = p.mid)
        }
        Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(20.dp)) {
            Text(androidx.compose.ui.text.buildAnnotatedString {
                pushStyle(androidx.compose.ui.text.SpanStyle(color = p.ink, fontWeight = FontWeight.Medium)); append("${model.threads.count { it.state == "working" }}"); pop(); append(" working")
            }, style = type(TypeScale.body, 1.4f), color = p.mid)
            Text(androidx.compose.ui.text.buildAnnotatedString {
                pushStyle(androidx.compose.ui.text.SpanStyle(color = p.ink, fontWeight = FontWeight.Medium)); append("${model.threads.count { it.state == "waiting" }}"); pop(); append(" waiting")
            }, style = type(TypeScale.body, 1.4f), color = p.mid)
        }
    }
    Box(Modifier.padding(horizontal = 24.dp).fillMaxWidth().height(1.dp).background(p.hair))
}

/** A row of the panel's: an icon, a name, a count at the right. */
@Composable
private fun PanelRow(icon: ImageVector, label: String, count: String?, go: () -> Unit) {
    val p = Ink
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp).clip(RoundedCornerShape(Radii.pill)).clickable(onClick = go).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) { Glyph(icon, 20.dp, p.mid) }
        Text(label, style = type(TypeScale.body, 1.4f), color = p.ink, modifier = Modifier.weight(1f))
        if (count != null) Text(count, style = Type.note, color = p.soft)
    }
}

/** Every other host this phone knows (App.tsx HostRows, hosts.tsx Hosts):
 * a press switches to it, the cross forgets it. The one in use is never
 * forgotten from here, as on the page. */
@Composable
private fun OtherHosts(model: ChatModel) {
    val p = Ink
    model.hosts.filter { it.url != model.address }.forEach { h ->
        val seen = model.hostsSeen
        val down = h.url in seen && seen[h.url] == null
        Row(
            Modifier.fillMaxWidth().clickable(onClickLabel = "Switch to ${h.name}") { model.switchTo(h.url) }
                .padding(start = 20.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(8.dp).background(if (down) p.accent else if (h.url in seen) Active else p.soft, CircleShape))
            Column(Modifier.weight(1f).padding(start = 10.dp)) {
                Text(h.name, style = type(TypeScale.body, 1.45f), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(h.url.substringAfter("://"), style = Type.note, color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Text(if (down) "Down" else seen[h.url]?.let { "$it ms" } ?: "…", style = Type.note, color = if (down) p.accent else p.soft)
            Round(onClick = { model.forgetHost(h.url) }, size = 36.dp) { Glyph(Glyph.close, 14.dp, p.mid, label = "Forget ${h.name}") }
        }
    }
}

/** What the app calls each of the memory's states. */
private val MEMORY_WORD = mapOf("working" to "Working", "catching_up" to "Catching up", "paused" to "Paused", "recording" to "Recording", "failing" to "Failing", "unknown" to "Unknown")

/** A fact of the host's: its name on the left, its value in --soft. */
@Composable
fun Fact(label: String, value: String) = SettingRow(label, value, chevron = false)

/** Seconds as the longest unit or two that matter, as the page says them. */
fun lasted(s: Long): String = when {
    s < 60 -> "$s seconds"
    s < 3600 -> "${s / 60} minutes"
    s < 7200 -> "1 hour"
    s < 86400 -> "${s / 3600} hours"
    s < 172800 -> "1 day"
    else -> "${s / 86400} days"
}

/** The categories, each only when it has something to set or see, in the
 * page's order, with what each is set to. */
@Composable
private fun SettingsList(model: ChatModel, pins: Pins, go: (String) -> Unit) {
    var links by remember { mutableStateOf<Connections?>(null) }
    LaunchedEffect(Unit) { links = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.connections() } }.getOrNull() } }
    var names by remember { mutableStateOf<WorkerNames?>(null) }
    LaunchedEffect(Unit) { names = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.workerNames() } }.getOrNull() } }
    var guide by remember { mutableStateOf<Guide?>(null) }
    LaunchedEffect(Unit) { guide = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.guide() } }.getOrNull() } }
    var hostGuide by remember { mutableStateOf<Guide?>(null) }
    LaunchedEffect(Unit) { hostGuide = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.hostGuide() } }.getOrNull() } }
    val theme = when (model.theme) { "light" -> "Light"; "dark" -> "Dark"; else -> "As the phone" }
    val m = model.models
    SettingRow("Appearance", "$theme · ${model.textPx}px", onClick = { go("Appearance") })
    SettingRow("Chat", if (pins.on.value) "Pinned on" else "Pinned off", onClick = { go("Chat") })
    if (model.me?.models == true) SettingRow("Models", m?.offers?.find { it.harness == m.harness && it.model == m.model }?.name.orEmpty(), onClick = { go("Models") })
    SettingRow("Usage", model.usage?.accounts?.joinToString(", ") { it.name }?.ifEmpty { null } ?: "Spend", onClick = { go("Usage") })
    names?.let { n -> SettingRow("Workers", if (n.allowed > 0u) "${n.allowed} names" else "Numbered", onClick = { go("Workers") }) }
    guide?.let { g -> SettingRow("Rules", if (g.text.isNotEmpty()) "Yours" else "None", onClick = { go("Guide") }) }
    hostGuide?.let { g -> SettingRow("Host facts", if (g.text.isNotEmpty()) "Written" else "None", onClick = { go("HostGuide") }) }
    links?.let { l ->
        val on = l.servers.filter { it.on }.map { it.name } + listOfNotNull(l.xai?.let { "xAI" })
        SettingRow("Connections", on.joinToString(", ").ifEmpty { "None" }, onClick = { go("Connections") })
    }
    SettingRow("Notifications", if (Bell.on(LocalContext.current)) "On" else "Off", onClick = { go("Notifications") })
    SettingRow("Voice", if (model.me?.voice != null) "${(model.voice ?: model.me?.voice).orEmpty().replaceFirstChar { it.uppercase() }} · ${"%.1f".format(model.voiceSpeed)}×" else "Dictation", onClick = { go("Voice") })
    if (model.memory != null || model.me?.forgets.orEmpty().isNotEmpty()) SettingRow("Memory", model.memory?.let { MEMORY_WORD[it.state] ?: it.state }, onClick = { go("Memory") })
    SettingRow("Hosts", model.info?.name, onClick = { go("Hosts") })
}

/** The person's standing rules, or with `host` what is true of this host
 * alone (App.tsx GuideEditor). The host holds them (guide.rs), so every
 * device shows the same text. A save the host refuses says why here and
 * keeps what it held. */
@Composable
private fun GuidePage(model: ChatModel, host: Boolean) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var guide by remember { mutableStateOf<Guide?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    // The text as being edited; null until a keystroke, so the box shows
    // what the host holds.
    var typed by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        runCatching { withContext(Dispatchers.IO) { model.host?.let { h -> if (host) h.hostGuide() else h.guide() } } }
            .onSuccess { guide = it }
            .onFailure { said = it.message ?: "The host didn't answer." }
    }
    val note = Modifier.padding(horizontal = 20.dp, vertical = 2.dp)
    Head(if (host) "Host facts" else "Rules", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    said?.let { Text(it, style = Type.note, color = p.accent, modifier = note) }
    val g = guide ?: return
    Text(
        if (host) "Facts about this host, for the main chat, Temp and workers."
        else "Every model reads these each turn. Calls read only the # Identity section.",
        style = Type.note, color = p.mid, modifier = note,
    )
    Column(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Field(typed ?: g.text, { typed = it }, if (host) "This host's facts" else "Rules for every model", lines = 16)
        Pressable("Save", enabled = !busy && typed != null) {
            val h = model.host ?: return@Pressable
            val text = typed.orEmpty()
            said = null
            busy = true
            scope.launch {
                runCatching { withContext(Dispatchers.IO) { if (host) h.setHostGuide(text) else h.setGuide(text) } }
                    .onSuccess { guide = it; typed = null }
                    .onFailure { said = it.message ?: "The host didn't answer." }
                busy = false
            }
        }
    }
}

/** The model that writes workers' titles and summaries (App.tsx
 * SynopsisModel): the cheapest the host can run, or any on offer. `now` is
 * null on a host without the choice: no row, and no sentence for it. A
 * failed pick says why under the row and keeps the old one. */
@Composable
private fun SynopsisPicker(model: ChatModel, offers: List<Offer>, now: SynopsisModel?, onNow: (SynopsisModel) -> Unit) {
    val scope = rememberCoroutineScope()
    var said by remember { mutableStateOf<String?>(null) }
    val p = Ink
    now?.let { s ->
        val name = { r: ModelRef -> offers.find { it.harness == r.harness && it.model == r.model }?.name ?: r.model }
        val default = "Default" + s.cheapest?.let { " (${name(it)})" }.orEmpty()
        val sorted = offers.sortedBy { offers.map { o -> o.provider }.distinct().indexOf(it.provider) }
        Choose(
            "Worker titles", s.picked?.let(name) ?: default, listOf<Offer?>(null) + sorted,
            name = { o -> o?.name ?: default },
            picked = { o -> o?.harness == s.picked?.harness && o?.model == s.picked?.model },
            group = { o -> o?.let { PROVIDER_NAMES[it.provider] ?: it.provider } },
        ) { o ->
            said = null
            scope.launch {
                val h = model.host
                runCatching { withContext(Dispatchers.IO) { h?.pickSynopsisModel(o?.let { ModelRef(it.harness, it.model) }) } }
                    .onSuccess { r -> r?.let(onNow) }
                    .onFailure { said = it.message ?: "That failed." }
            }
        }
    }
    said?.let { Text(it, style = Type.note, color = p.accent, modifier = Modifier.padding(horizontal = 20.dp, vertical = 2.dp)) }
}

/** A name that a tap excludes or brings back, or, given `remove`, one with a
 * cross to take it off the list (App.tsx NameChip). */
@Composable
private fun NameChip(name: String, off: Boolean, enabled: Boolean, tap: (() -> Unit)? = null, remove: (() -> Unit)? = null) {
    val p = Ink
    val look = Type.note.copy(textDecoration = if (off) TextDecoration.LineThrough else null)
    Row(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).border(1.dp, p.frame, CircleShape)
            .then(if (tap != null) Modifier.clickable(enabled = enabled, onClickLabel = if (off) "Bring $name back" else "Exclude $name", onClick = tap) else Modifier)
            .padding(start = 12.dp, end = if (remove != null) 4.dp else 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(name, style = look, color = if (off) p.mid else p.ink)
        remove?.let {
            Box(Modifier.padding(start = 2.dp).size(28.dp).clip(CircleShape).clickable(enabled = enabled, onClickLabel = "Remove $name", onClick = it), contentAlignment = Alignment.Center) {
                Glyph(Glyph.close, 14.dp, p.mid)
            }
        }
    }
}

/** The names new workers are given, as the host keeps them (App.tsx
 * WorkerNames), so every device shows the same list. A change the host
 * refuses says why and leaves the list as it was. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun WorkersPage(model: ChatModel) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var names by remember { mutableStateOf<WorkerNames?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var typed by remember { mutableStateOf("") }
    // Their own list as being edited; null until a keystroke, so the box
    // shows what the host holds.
    var pasted by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        runCatching { withContext(Dispatchers.IO) { model.host?.workerNames() } }
            .onSuccess { names = it }
            .onFailure { said = it.message ?: "The host didn't answer." }
    }
    fun change(then: () -> Unit = {}, ask: (Host) -> WorkerNames) {
        val h = model.host ?: return
        said = null
        busy = true
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { ask(h) } }
                .onSuccess { names = it; then() }
                .onFailure { said = it.message ?: "The host didn't answer." }
            busy = false
        }
    }
    val heading = Modifier.padding(start = 20.dp, top = 12.dp, bottom = 4.dp)
    val spread = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)
    val gap = Arrangement.spacedBy(6.dp)
    val note = Modifier.padding(horizontal = 20.dp, vertical = 2.dp)
    Head("Names", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    said?.let { Text(it, style = Type.note, color = p.accent, modifier = note) }
    val n = names ?: return
    val excluded = { name: String -> n.excluded.any { it.equals(name, ignoreCase = true) } }
    val sway = { name: String ->
        val off = excluded(name)
        change { it.changeWorkerName(if (off) "restore" else "exclude", name) }
    }
    SettingRow("Use my own list", chevron = false) { PaperSwitch(n.useOwn) { on -> if (!busy) change { it.useOwnWorkerNames(on) } } }
    Text(if (n.useOwn) "Your list replaces the built-in names." else "Tap a name to leave it out or bring it back.", style = Type.note, color = p.mid, modifier = note)
    if (n.allowed == 0u) Text("No names left; new workers are numbered.", style = Type.note, color = p.mid, modifier = note)
    if (n.useOwn) {
        Column(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(pasted ?: n.own.joinToString("\n"), { pasted = it }, "Names, with commas or one per line", lines = 8)
            Pressable("Save list", enabled = !busy && pasted != null) {
                val text = pasted.orEmpty()
                change(then = { pasted = null }) { it.setOwnWorkerNames(text) }
            }
        }
        if (n.leftOut.isNotEmpty()) Text("Left out (one word, letters only): ${n.leftOut.joinToString(", ")}.", style = Type.note, color = p.mid, modifier = note)
        if (n.own.isNotEmpty()) {
            Text("Your list · ${n.own.size}", style = Type.note, color = p.mid, modifier = heading)
            FlowRow(spread, horizontalArrangement = gap, verticalArrangement = gap) {
                n.own.forEach { name -> NameChip(name, excluded(name), !busy, tap = { sway(name) }) }
            }
        }
    } else {
        val find = typed.trim()
        val found = { name: String -> name.contains(find, ignoreCase = true) }
        val added = n.added.filter(found)
        val builtin = n.builtin.filter(found)
        Row(Modifier.padding(horizontal = 20.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Field(typed, { typed = it.replace("\n", "") }, "Find or add a name", Modifier.weight(1f), lines = 1)
            Pressable("Add", enabled = !busy && find.isNotEmpty()) {
                change(then = { typed = "" }) { it.changeWorkerName("add", find) }
            }
        }
        if (added.isNotEmpty()) {
            Text("Added · ${added.size}", style = Type.note, color = p.mid, modifier = heading)
            FlowRow(spread, horizontalArrangement = gap, verticalArrangement = gap) {
                added.forEach { name ->
                    NameChip(name, excluded(name), !busy, remove = { change { it.changeWorkerName("remove", name) } })
                }
            }
        }
        Text("Built-in · ${builtin.size}", style = Type.note, color = p.mid, modifier = heading)
        FlowRow(spread, horizontalArrangement = gap, verticalArrangement = gap) {
            builtin.forEach { name -> NameChip(name, excluded(name), !busy, tap = { sway(name) }) }
        }
    }
}

/** A setting picked from a list, as the page's select: the choice in
 * --soft at the right; a tap opens the list as a menu. */
@Composable
private fun <T> Choose(label: String, now: String, options: List<T>, name: (T) -> String, picked: (T) -> Boolean, group: ((T) -> String?)? = null, pick: (T) -> Unit) {
    val p = Ink
    var open by remember { mutableStateOf(false) }
    Box {
        SettingRow(label, chevron = false, onClick = { open = true }) {
            Text(now, style = Type.note, color = p.mid, maxLines = 1, modifier = Modifier.widthIn(max = 180.dp))
            Glyph(Glyph.chevronDown, 13.dp, p.soft, Modifier.padding(start = 4.dp))
        }
        PaperMenu(open, { open = false }, modifier = Modifier.heightIn(max = 420.dp)) {
            var last: String? = null
            options.forEach { o ->
                group?.invoke(o)?.let { g -> if (g != last) { Head(g, Modifier.padding(start = 16.dp, top = 8.dp, bottom = 2.dp)); last = g } }
                MenuRow(name(o), picked = picked(o), onClick = { open = false; pick(o) })
            }
        }
    }
}

private val HARNESS_NAMES = mapOf("claude" to "Claude Code", "codex" to "Codex", "muse" to "Muse Code", "opencode" to "OpenCode", "omp" to "omp", "pi" to "Pi", "grok" to "Grok Build")
private val PROVIDER_NAMES = mapOf(
    "anthropic" to "Anthropic", "openai" to "OpenAI", "google" to "Google", "xai" to "xAI",
    "deepseek" to "DeepSeek", "opencode" to "OpenCode", "mistral" to "Mistral", "moonshotai" to "Moonshot",
    "meta" to "Meta",
)

/** One choice of the Workers row: its words, the model, none being "Same as
 * Chat", and the head it sits under. */
private class WorkerRow(val name: String, val model: ModelRef?, val group: String? = null)

/** Models (App.tsx Models): the default model for each kind of making, and
 * the harness the other makers' models run in. */
@Composable
private fun ModelsPage(model: ChatModel) {
    OmpSignIn(model)
    val p = Ink
    var makers by remember { mutableStateOf<Makers?>(null) }
    val scope = rememberCoroutineScope()
    val ask = { scope.launch { makers = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.makers() } }.getOrNull() } } }
    LaunchedEffect(Unit) { model.refresh(); ask() }
    val m = model.models
    // Null on a host without the choice: no row, and no sentence for it.
    var workers by remember { mutableStateOf<WorkerModel?>(null) }
    var workersSaid by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        runCatching { withContext(Dispatchers.IO) { model.host?.workerModel() } }
            .onSuccess { workers = it }
            .onFailure { workersSaid = it.message ?: "The host didn't answer." }
    }
    var synopsis by remember { mutableStateOf<SynopsisModel?>(null) }
    LaunchedEffect(Unit) {
        synopsis = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.synopsisModel() } }.getOrNull() }
    }
    Head("Defaults", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    if (m != null && m.offers.isNotEmpty()) {
        Choose(
            "Chat", m.offers.find { it.harness == m.harness && it.model == m.model }?.name ?: m.model,
            m.offers.sortedBy { m.offers.map { o -> o.provider }.distinct().indexOf(it.provider) },
            name = { it.name }, picked = { it.harness == m.harness && it.model == m.model },
            group = { PROVIDER_NAMES[it.provider] ?: it.provider },
        ) { model.pickModel(it.harness, it.model) }
    }
    if (m != null && m.offers.isNotEmpty()) workers?.let { w ->
        val held = w.picked
        // Off the list it stays the pick, under "Same as Chat", and new
        // workers start on Chat's model until it returns.
        val off = held?.takeIf { h -> m.offers.none { it.harness == h.harness && it.model == h.model } }
        val rows = listOfNotNull(WorkerRow("Same as Chat", null), off?.let { WorkerRow("Not offered: ${it.model}", it) }) +
            m.offers.sortedBy { m.offers.map { o -> o.provider }.distinct().indexOf(it.provider) }
                .map { WorkerRow(it.name, ModelRef(it.harness, it.model), PROVIDER_NAMES[it.provider] ?: it.provider) }
        Choose(
            "Workers", rows.first { it.model == held }.name, rows,
            name = { it.name }, picked = { it.model == held }, group = { it.group },
        ) { c ->
            if (c.model == held) return@Choose
            workersSaid = null
            scope.launch {
                val h = model.host
                runCatching { withContext(Dispatchers.IO) { h?.pickWorkerModel(c.model) } }
                    .onSuccess { workers = it ?: workers }
                    .onFailure { workersSaid = it.message ?: "That failed." }
            }
        }
    }
    workersSaid?.let { Text(it, style = Type.note, color = p.accent, modifier = Modifier.padding(horizontal = 20.dp, vertical = 2.dp)) }
    if (m != null && m.offers.isNotEmpty()) SynopsisPicker(model, m.offers, synopsis) { synopsis = it }
    makers?.let { mk ->
        if (mk.pictures.isNotEmpty()) {
            Choose(
                "Image generation", mk.picked?.name.orEmpty(), mk.pictures, name = { it.name },
                picked = { it.provider == mk.picked?.provider && it.model == mk.picked?.model },
            ) { c -> model.act(then = { ask() }) { model.host?.pickPictures(c.provider, c.model) } }
        }
        mk.fixed.forEach { Fact(it.capability, it.name) }
    }
    Text("The picker under the message box changes Chat too." + if (workers != null) " Workers keep the model they start on." else "", style = Type.note, color = p.mid, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))
    if (m != null && m.modes.isNotEmpty()) {
        Head("Permissions", Modifier.padding(start = 20.dp, top = 18.dp, bottom = 4.dp))
        m.modes.forEach { h ->
            val title = HARNESS_NAMES[h.harness] ?: h.harness
            Choose(title, h.access.find { it.id == h.defaultAccess }?.name ?: h.defaultAccess, h.access, name = { it.name }, picked = { it.id == h.defaultAccess }) {
                model.pickAccessDefault(h.harness, it.id, null)
            }
            if (h.sandbox.isNotEmpty()) {
                Choose("$title sandbox", h.sandbox.find { it.id == h.defaultSandbox }?.name.orEmpty(), h.sandbox, name = { it.name }, picked = { it.id == h.defaultSandbox }) {
                    model.pickAccessDefault(h.harness, null, it.id)
                }
            }
        }
        Text("Where each chat starts; the picker under the message box changes it per chat.", style = Type.note, color = p.mid, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))
    }
    if (m != null && (m.harnesses.isNotEmpty() || m.othersOffered.size > 1)) {
        Head("Harnesses", Modifier.padding(start = 20.dp, top = 18.dp, bottom = 4.dp))
        m.harnesses.forEach { h ->
            val name = HARNESS_NAMES[h.harness] ?: h.harness
            val out = h.on && h.harness in m.signedOut
            val default = m.defaultHarness == h.harness
            // The host refuses the last one off; the default moves only by picking another.
            val held = h.on && (default || m.harnesses.count { it.on } == 1)
            Box(Modifier.alpha(if (out) 0.5f else 1f)) {
                SettingRow(name, chevron = false) {
                    if (h.on) {
                        Text(
                            "Default", style = type(TypeScale.label, 1.3f), color = if (default) p.paper else p.mid,
                            modifier = Modifier.padding(end = 8.dp).clip(CircleShape).background(if (default) p.ink else Color.Transparent)
                                .clickable(enabled = !default && !out) { model.act(then = model::refresh) { model.host?.defaultHarness(h.harness) } }
                                .padding(horizontal = 10.dp, vertical = 4.dp),
                        )
                    }
                    PaperSwitch(h.on, !held) { on -> model.act(then = model::refresh) { model.host?.switchHarness(h.harness, on) } }
                }
            }
            if (h.on && !out) {
                val own = m.offers.filter { it.harness == h.harness }
                // A default model the harness no longer offers stays a row of its own, as on the web.
                val stale = h.model.isNotEmpty() && own.none { it.model == h.model }
                val rows: List<String?> = listOf(null) + (if (stale) listOf(h.model) else emptyList()) + own.map { it.model }
                Choose(
                    "$name default model", own.find { it.model == h.model }?.name ?: h.model.ifEmpty { "Its own" }, rows,
                    name = { r -> if (r == null) "Its own" else own.find { it.model == r }?.name ?: r }, picked = { (it ?: "") == h.model },
                ) { model.act(then = model::refresh) { model.host?.defaultModel(h.harness, it ?: "") } }
            }
            if (out) {
                Text("Not signed in: ${h.remedy.removeSuffix(".")}.", style = Type.note, color = p.accent, modifier = Modifier.padding(horizontal = 20.dp, vertical = 2.dp))
            }
        }
        if (m.othersOffered.size > 1) {
            SettingRow("Other models via", chevron = false) {
                Row(Modifier.border(1.25.dp, p.edge, CircleShape).padding(2.dp)) {
                    m.othersOffered.forEach { h ->
                        val on = m.others == h
                        val out = h in m.signedOut
                        Text(
                            HARNESS_NAMES[h] ?: h, style = type(TypeScale.label, 1.3f), color = if (on) p.paper else p.mid,
                            modifier = Modifier.clip(CircleShape).background(if (on) p.ink else Color.Transparent)
                                .alpha(if (out && !on) 0.5f else 1f)
                                .clickable(enabled = !on && !out) { model.act(then = model::refresh) { model.host?.pickOthers(h) } }
                                .padding(horizontal = 12.dp, vertical = 4.dp),
                        )
                    }
                }
            }
        }
        Text("Only harnesses that are on show in the pickers.", style = Type.note, color = p.mid, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp))
    }
}

/** Connections (connections.tsx): MCP servers by name and address, which
 * every harness on the host shares. A server's OAuth sign-in opens in the
 * browser and returns it to the host's own loopback address, which fails
 * to load on a phone: its address is pasted here to finish. */
@Composable
private fun ConnectionsPage(model: ChatModel) {
    val p = Ink
    var now by remember { mutableStateOf<Connections?>(null) }
    var editing by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val load = {
        scope.launch {
            model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.connections() } }.onSuccess { now = it } }
        }
    }
    LaunchedEffect(Unit) { load() }
    // Built-ins hear of a change to the connections, which may fill them.
    var turn by remember { mutableStateOf(0) }
    val changed = { load(); turn++; Unit }
    Head("Connections", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    val links = now
    if (links == null) { Small("Asking…", Modifier.padding(horizontal = 20.dp)); return }
    if (!model.readOnly) Providers(model, turn, changed)
    links.servers.forEach { s ->
        if (editing == s.name) ServerForm(model, s) { editing = null; changed() }
        else ServerRow(model, s, { editing = s.name }) { changed() }
    }
    HostServers(model)
    // A device that may only read is refused by the host, which says so.
    if (editing == "") ServerForm(model, null) { editing = null; changed() }
    else Box(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) { Pressable("Add another MCP server") { editing = "" } }
    Builtins(model, turn) { changed() }
    Head("xAI", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    XaiConnection(model, links) { load() }
    OtherApps(model)
}

/** The servers this host's deployment adds to every turn (connections.tsx
 * HostServers): a row each with its tool count, and no switch, edit or
 * remove, as they live in the host's own setup. */
@Composable
private fun HostServers(model: ChatModel) {
    val p = Ink
    var all by remember { mutableStateOf<List<su.libri.core.HostServer>>(emptyList()) }
    LaunchedEffect(Unit) {
        model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.hostServers() } }.onSuccess { all = it } }
    }
    all.forEach { s ->
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(s.name, style = type(TypeScale.body, 1.45f), color = p.ink)
                Text("Added by this host", style = Type.note, color = p.mid, maxLines = 1)
            }
            s.tools?.let { Text(if (it == 1u) "1 tool" else "$it tools", style = Type.note, color = p.soft, maxLines = 1) }
        }
    }
}

/** What to paste where (connections.tsx pasteText): an app that signs in by
 * OAuth takes the address alone; one that takes a header gets the key too. */
private fun pasteText(url: String, key: String?): List<Triple<String, String, String?>> =
    if (key == null) listOf(
        Triple("Claude, ChatGPT, Cursor and other connector apps", url,
            "Add it as a custom connector, then sign in to Libri."),
        Triple("Claude Code", "claude mcp add --transport http libri $url", null),
        Triple("Codex", "codex mcp add libri --url $url", null),
    ) else listOf(
        Triple("Claude Code", "claude mcp add --transport http libri $url --header \"Authorization: Bearer $key\"", null),
        Triple("Apps that take a header", "{\"mcpServers\":{\"libri\":{\"url\":\"$url\",\"headers\":{\"Authorization\":\"Bearer $key\"}}}}", null),
        Triple("Apps that start a command, on this host", "{\"mcpServers\":{\"libri\":{\"command\":\"libri\",\"args\":[\"mcp\"],\"env\":{\"LIBRI_CONNECTOR_KEY\":\"$key\"}}}}", null),
    )

/** The apps let in at /mcp (connections.tsx OtherApps): each connector with
 * its own sign-in and the modules it may use, revoked here; and how to add
 * one. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun OtherApps(model: ChatModel) {
    val p = Ink
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboard.current
    var door by remember { mutableStateOf<su.libri.core.OtherApps?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    var made by remember { mutableStateOf<su.libri.core.ConnectorMade?>(null) }
    var revoking by remember { mutableStateOf<su.libri.core.Connector?>(null) }
    val read = model.readOnly
    val load = {
        scope.launch {
            model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.otherApps() } }.onSuccess { door = it } }
        }
    }
    LaunchedEffect(Unit) { load() }
    val act = { work: suspend () -> Unit ->
        said = null
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { work() } }
                .onSuccess { load() }.onFailure { said = it.message ?: "That failed." }
        }
    }
    val d = door ?: return
    val url = d.url
    val copy = { label: String, text: String ->
        scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(label, text))); said = "Copied." }
        Unit
    }
    val paste = @Composable { items: List<Triple<String, String, String?>> ->
        items.forEach { (app, text, step) ->
            Small(app, Modifier.padding(start = 20.dp, top = 6.dp))
            Text(
                text, style = type(TypeScale.caption, 1.4f, family = Faces.mono), color = p.ink,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 2.dp).clip(RoundedCornerShape(Radii.lg))
                    .background(p.block).heightIn(min = 44.dp)
                    .clickable { copy(app, text) }
                    .padding(horizontal = 12.dp, vertical = 10.dp),
            )
            step?.let { Small(it, Modifier.padding(horizontal = 20.dp)) }
        }
    }
    Head("Other apps", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    Small("Apps that use Libri over MCP, each limited to the parts you allow.",
        Modifier.padding(horizontal = 20.dp))
    d.connectors.forEach { c ->
        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(start = 20.dp, end = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(end = 10.dp)) {
                Text(c.name, style = type(TypeScale.body, 1.45f), color = p.ink)
                val used = if (c.used == 0UL) "never" else java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM, java.text.DateFormat.SHORT)
                    .format(java.util.Date(c.used.toLong() * 1000))
                Text("${c.app?.let { "Signed in by $it" } ?: "By key"} · used $used", style = Type.note, color = p.mid)
            }
            if (!read) Quiet("Revoke", p.mid) { revoking = c }
        }
        FlowRow(Modifier.padding(horizontal = 20.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            d.modules.forEach { m ->
                val on = m.id in c.modules
                Text(
                    m.title, style = type(TypeScale.caption, 1.3f), color = if (on) p.ink else p.mid,
                    modifier = Modifier.heightIn(min = 36.dp).clip(CircleShape).border(1.25.dp, if (on) p.ink else p.frame, CircleShape)
                        .semantics { contentDescription = "${c.name}: ${m.title} ${if (on) "on" else "off"}" }
                        .clickable(enabled = !read) {
                            act { model.host!!.connectorSet(c.id, if (on) c.modules - m.id else c.modules + m.id) }
                        }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                )
            }
        }
    }
    made?.let { m ->
        Text("Key for ${m.connector.name}", style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.padding(start = 20.dp, top = 12.dp))
        Small("Shown once. Paste it where the app takes a header.", Modifier.padding(horizontal = 20.dp))
        paste(pasteText(url, m.key))
        Box(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) { Pressable("Done") { made = null } }
    }
    if (adding) {
        var name by remember { mutableStateOf("") }
        var picked by remember { mutableStateOf(d.modules.filter { it.start }.map { it.id }) }
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Field(name, { name = it }, "Name, as Claude Code on the laptop", lines = 1)
            d.modules.forEach { m ->
                Row(Modifier.fillMaxWidth().heightIn(min = 52.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f).padding(end = 10.dp)) {
                        Text(m.title, style = type(TypeScale.body, 1.45f), color = p.ink)
                        Text(m.about, style = Type.note, color = p.mid)
                    }
                    Box(Modifier.semantics { contentDescription = m.title }) {
                        PaperSwitch(m.id in picked) { picked = if (it) picked + m.id else picked - m.id }
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Pressable("Add", filled = true, enabled = name.isNotBlank()) {
                    said = null
                    scope.launch {
                        runCatching { withContext(Dispatchers.IO) { model.host!!.connectorAdd(name.trim(), picked) } }
                            .onSuccess { made = it; adding = false; load() }.onFailure { said = it.message ?: "That failed." }
                    }
                }
                Quiet("Cancel", p.mid) { adding = false }
            }
        }
    } else if (!read && made == null) {
        Box(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) { Pressable("Add by key") { adding = true } }
    }
    Text("Connect an app", style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.padding(start = 20.dp, top = 12.dp))
    paste(pasteText(url, null))
    said?.let { Small(it, Modifier.padding(horizontal = 20.dp), color = p.ink) }
    revoking?.let { c ->
        ConfirmDialog(
            title = "Revoke ${c.name}?",
            body = "${c.app ?: "The app"} is signed out at once, and its key stops working.",
            confirm = "Revoke",
            danger = true,
            onDismiss = { revoking = null },
            onConfirm = { revoking = null; act { model.host!!.connectorRevoke(c.id) } },
        )
    }
}

/** The built-ins, Mail and Calendar (connections.tsx Builtins): each a
 * switch, filled on its own by the connection a provider covers it with,
 * with a pick only where two connected accounts both cover it. */
@Composable
private fun Builtins(model: ChatModel, turn: Int, changed: () -> Unit) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var all by remember { mutableStateOf<List<Builtin>?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    val read = model.readOnly
    val get = {
        scope.launch {
            model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.builtins() } }.onSuccess { all = it; model.builtins = it } }
        }
    }
    LaunchedEffect(turn) { get() }
    val put = { id: String, on: Boolean?, provider: String? ->
        said = null
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { model.host!!.builtinSet(id, on, provider) } }
                .onFailure { said = it.message ?: "That failed." }
            get(); changed()
        }
    }
    val list = all ?: return
    Head("Built-ins", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    list.forEach { b ->
        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(b.title, style = type(TypeScale.body, 1.45f), color = p.ink)
                Text(
                    b.connection?.let { "From ${b.providerTitle}" + if (b.signedIn) "" else " · not signed in" } ?: "No connection yet: add one above",
                    style = Type.note, color = p.mid,
                )
            }
            Box(Modifier.semantics { contentDescription = "${b.title} on" }) {
                PaperSwitch(b.on, !read) { on -> put(b.id, on, null) }
            }
        }
        if (b.on && b.choices.size > 1) {
            Row(Modifier.padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                b.choices.forEach { c ->
                    Quiet(c.title, if (c.id == b.provider) p.ink else p.mid) { if (!read) put(b.id, null, c.id) }
                }
            }
        }
    }
    said?.let { Small(it, Modifier.padding(horizontal = 20.dp), color = p.ink) }
}

/** The providers Libri can connect (connections.tsx Providers), as a
 * phone's Add account: one tap adds the connection and opens its sign-in,
 * and every built-in it covers takes it. */
@Composable
private fun Providers(model: ChatModel, turn: Int, changed: () -> Unit) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var all by remember { mutableStateOf<List<su.libri.core.Provider>?>(null) }
    LaunchedEffect(turn) {
        model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.providers() } }.onSuccess { all = it } }
    }
    val open = all.orEmpty().filter { !it.signedIn }
    if (open.isEmpty()) return
    Small("Add a connection", Modifier.padding(start = 20.dp, top = 2.dp))
    open.forEach { pr ->
        var name by remember(pr.id) { mutableStateOf(pr.connection.orEmpty()) }
        var said by remember(pr.id) { mutableStateOf<String?>(null) }
        val fail = { e: Throwable -> said = e.message ?: "That failed." }
        val sign = rememberSignIn(model, name, pr.signedIn, changed, fail)
        Row(
            Modifier.fillMaxWidth().heightIn(min = 52.dp).clickable {
                said = null
                scope.launch {
                    runCatching { withContext(Dispatchers.IO) { model.host!!.providerConnect(pr.id) } }
                        .onSuccess { made -> name = made; changed(); sign.startAs(made) }
                        .onFailure(fail)
                }
            }.padding(horizontal = 20.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(if (pr.connection != null) "Sign in to ${pr.title}" else pr.title, style = type(TypeScale.body, 1.45f), color = p.ink)
                Text(pr.covers.joinToString(", "), style = Type.note, color = p.mid)
            }
            Text(if (pr.connection != null) "Sign in" else "Connect", style = Type.note, color = p.mid)
        }
        sign.Paste()
        said?.let { Small(it, Modifier.padding(horizontal = 20.dp)) }
    }
}

/** A connection's tools, each Allow, Ask or Off (connections.tsx Tools),
 * folded to a count until opened. */
@Composable
private fun Tools(model: ChatModel, s: Server, load: () -> Unit) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var open by remember(s.name) { mutableStateOf(false) }
    if (s.tools.isEmpty()) return
    val asks = s.tools.count { it.rule == "ask" }
    val off = s.tools.count { it.rule == "off" }
    Text(
        "${s.tools.size} tools" + (if (asks > 0) " · $asks ask first" else "") + (if (off > 0) " · $off off" else "") + if (open) "  ▴" else "  ▾",
        style = Type.note, color = p.mid,
        modifier = Modifier.fillMaxWidth().heightIn(min = 44.dp).clickable { open = !open }.padding(horizontal = 20.dp, vertical = 12.dp),
    )
    if (!open) return
    s.tools.forEach { t ->
        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 20.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(end = 10.dp)) {
                Text(t.name, style = type(TypeScale.caption, 1.4f, family = Faces.mono), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                t.description?.let { Text(it, style = Type.note, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
            Row(Modifier.clip(RoundedCornerShape(Radii.pill)).border(1.dp, p.frame, RoundedCornerShape(Radii.pill)).padding(2.dp)) {
                listOf("allow" to "Allow", "ask" to "Ask", "off" to "Off").forEach { (r, label) ->
                    val on = t.rule == r
                    Box(
                        Modifier.heightIn(min = 40.dp).clip(RoundedCornerShape(Radii.pill)).background(if (on) p.ink else Color.Transparent)
                            .semantics { contentDescription = "${t.name} $label"; selected = on }
                            .clickable(enabled = !model.readOnly && !on) {
                                scope.launch {
                                    runCatching { withContext(Dispatchers.IO) { model.host!!.toolRule(s.name, t.name, r) } }
                                    load()
                                }
                            }.padding(horizontal = 10.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(label, style = Type.note, color = if (on) p.paper else p.mid)
                    }
                }
            }
        }
    }
}

/** What a connection brings in (connections.tsx Brings), each shown or
 * hidden: the Calendar view and the agent's searches leave out a hidden one. */
@Composable
private fun Brings(model: ChatModel, s: Server) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var kinds by remember(s.name) { mutableStateOf<List<su.libri.core.ItemKind>>(emptyList()) }
    val get = {
        scope.launch {
            model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.items(s.name) } }.onSuccess { kinds = it } }
        }
    }
    // A server run here needs no sign-in.
    LaunchedEffect(s.name, s.signedIn, s.on) { if ((s.signedIn || !s.remote) && s.on) get() }
    kinds.forEach { k ->
        Small(k.title, Modifier.padding(start = 20.dp, top = 6.dp))
        k.failed?.let { Small(it, Modifier.padding(horizontal = 20.dp)) }
        k.items.forEach { i ->
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                val dot = i.color?.takeIf { it.startsWith("#") && it.length == 7 }?.let { Color(android.graphics.Color.parseColor(it)) }
                Box(Modifier.size(10.dp).clip(CircleShape).background(dot ?: Color.Transparent).border(1.dp, p.hair, CircleShape))
                Text(i.name, style = type(TypeScale.body, 1.4f), color = p.ink, modifier = Modifier.weight(1f).padding(start = 12.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                Box(Modifier.semantics { contentDescription = "Show ${i.name}" }) {
                    PaperSwitch(i.shown, !model.readOnly) { on ->
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { model.host!!.itemShow(s.name, k.kind, i.id, on) } }
                            get()
                        }
                    }
                }
            }
        }
    }
}

/** Signing a connection in (connections.tsx useSignIn): `start` opens the
 * server's page in the browser; `Paste` then shows, below the row, where the
 * address it ends on goes when the sign-in cannot come back by itself.
 * ServerRow and a built-in's chosen provider both use it. */
private class SignIn(val start: () -> Unit, val startAs: (String) -> Unit, val Paste: @Composable () -> Unit)

@Composable
private fun rememberSignIn(model: ChatModel, name: String, signedIn: Boolean, load: () -> Unit, fail: (Throwable) -> Unit): SignIn {
    val p = Ink
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var signing by remember(name) { mutableStateOf(false) }
    var address by remember(name) { mutableStateOf("") }
    // Back from the browser: a sign-in finished on the host itself shows.
    val owner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    androidx.compose.runtime.DisposableEffect(owner, signing) {
        val o = androidx.lifecycle.LifecycleEventObserver { _, e -> if (e == androidx.lifecycle.Lifecycle.Event.ON_RESUME && signing) load() }
        owner.lifecycle.addObserver(o)
        onDispose { owner.lifecycle.removeObserver(o) }
    }
    if (signedIn && signing) signing = false
    // Back in the app by its own scheme (ChatModel.signedIn): the host has
    // finished the sign-in, or said why not.
    val back = model.signedBack
    androidx.compose.runtime.LaunchedEffect(back) {
        if (back == 0 || !signing) return@LaunchedEffect
        model.signInSaid?.let { fail(Exception(it)) }
        load()
    }
    val begin = { who: String ->
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { model.host!!.serverSignIn(who) } }
                .onSuccess { url ->
                    context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)))
                    signing = true
                }.onFailure(fail)
        }
        Unit
    }
    return SignIn(
        start = { begin(name) },
        // A connection added by the same tap, before `name` knows it.
        startAs = begin,
        Paste = {
            if (signing && !signedIn) {
                Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text("Not back in Libri? Paste the address of the page you ended on.", style = Type.note, color = p.mid)
                    Field(address, { address = it }, "http://127.0.0.1:…/api/connections/back?code=…", lines = 3)
                    Pressable("Finish signing in", filled = true, enabled = address.isNotBlank()) {
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { model.host!!.serverFinish(address.trim()) } }
                                .onSuccess { signing = false; address = ""; load() }.onFailure(fail)
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun ServerRow(model: ChatModel, s: Server, edit: () -> Unit, load: () -> Unit) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var tested by remember(s.name) { mutableStateOf<Tested?>(null) }
    var testing by remember { mutableStateOf(false) }
    var said by remember { mutableStateOf<String?>(null) }
    var removing by remember { mutableStateOf(false) }
    val fail = { e: Throwable -> said = e.message ?: "That failed." }
    val sign = rememberSignIn(model, s.name, s.signedIn, load, fail)
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(s.name, style = type(TypeScale.body, 1.45f), color = p.ink)
            Text(s.address, style = Type.note, color = p.mid, maxLines = 1)
            val state = if (s.signedIn) s.account?.let { "Signed in as $it" } ?: "Signed in" else s.secrets.joinToString(", ").ifEmpty { null }
            state?.let { Text(it, style = Type.note, color = p.mid, maxLines = 1) }
        }
        // One switch for every on and off in Settings, as the built-ins' above.
        Box(Modifier.semantics { contentDescription = "${s.name} on" }) {
            PaperSwitch(s.on) { on ->
                scope.launch {
                    runCatching { withContext(Dispatchers.IO) { model.host!!.serverPut(s.name, s.address, null, s.withheld, on) } }
                        .onSuccess { load() }.onFailure(fail)
                }
            }
        }
    }
    Row(Modifier.padding(horizontal = 12.dp)) {
        Quiet(if (testing) "Testing…" else "Test", p.mid) {
            said = null; tested = null; testing = true
            scope.launch {
                runCatching { withContext(Dispatchers.IO) { model.host!!.serverTest(s.name) } }
                    // The test lists the tools through the relay, which keeps them for here.
                    .onSuccess { tested = it; load() }.onFailure(fail)
                testing = false
            }
        }
        if (s.remote) Quiet(if (s.signedIn) "Sign in again" else "Sign in", p.mid) { said = null; sign.start() }
        Quiet("Edit", p.mid, edit)
        Quiet("Remove", p.mid) { removing = true }
    }
    sign.Paste()
    tested?.let { t ->
        if (t.signIn) Small("${s.name} wants you to sign in first.", Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
        else if (s.tools.isEmpty()) Small("${t.tools.size} tools", Modifier.padding(horizontal = 20.dp, vertical = 4.dp))
    }
    Brings(model, s)
    Tools(model, s, load)
    said?.let { Text(it, style = Type.note, color = p.mid, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp)) }
    if (removing) {
        Sheet({ removing = false }) {
            Text("Remove ${s.name}?", style = type(TypeScale.prose, 1.3f, FontWeight.SemiBold), color = p.ink)
            Text("Its tools and sign-in are removed.", style = Type.small, color = p.mid, modifier = Modifier.padding(top = 4.dp, bottom = 14.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Quiet("Cancel", p.mid) { removing = false }
                Spacer(Modifier.width(6.dp))
                Pressable("Remove", filled = true) {
                    removing = false
                    scope.launch { runCatching { withContext(Dispatchers.IO) { model.host!!.serverRemove(s.name) } }.onSuccess { load() }.onFailure(fail) }
                }
            }
        }
    }
}

/** Adds a connection, or edits one; its secrets never come back from the
 * host, so an edit keeps them unless new ones are typed. */
@Composable
private fun ServerForm(model: ChatModel, was: Server?, done: () -> Unit) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf(was?.name.orEmpty()) }
    var address by remember { mutableStateOf(was?.address.orEmpty()) }
    var secrets by remember { mutableStateOf("") }
    var withheld by remember { mutableStateOf(was?.withheld?.joinToString(", ").orEmpty()) }
    var said by remember { mutableStateOf<String?>(null) }
    val remote = address.trim().let { it.startsWith("http://") || it.startsWith("https://") }
    Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (was == null) Field(name, { name = it }, "Name, as mail", lines = 1)
        else Text(was.name, style = type(TypeScale.body, 1.45f), color = p.ink)
        Field(address, { address = it }, "https://example.com/mcp or a command", lines = 2)
        Small("A URL, or a command run on this host.")
        Field(secrets, { secrets = it },
            if (was?.secrets?.isNotEmpty() == true) "Kept: ${was.secrets.joinToString(", ")}. Type to replace."
            else if (remote) "Headers, as Authorization: Bearer … (optional)" else "Environment, as API_KEY=… (optional)",
            lines = 2, secret = true)
        Field(withheld, { withheld = it }, "Withheld tools, as send_email (optional)", lines = 1)
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Pressable(if (was == null) "Add" else "Save", filled = true, enabled = name.isNotBlank() && address.isNotBlank()) {
                said = null
                scope.launch {
                    runCatching {
                        withContext(Dispatchers.IO) {
                            model.host!!.serverPut(name.trim(), address.trim(),
                                // Nothing typed on an edit keeps the kept ones.
                                if (secrets.isBlank() && was != null) null else secrets,
                                withheld.split(",").map { it.trim() }.filter { it.isNotEmpty() },
                                was?.on ?: true)
                        }
                    }.onSuccess { done() }.onFailure { said = it.message ?: "That failed." }
                }
            }
            Quiet("Cancel", p.mid, done)
        }
        said?.let { Small(it) }
    }
}

/** xAI (connections.tsx Xai), for read aloud, calls, X search and pictures:
 * a sign-in approved on any device, which the host waits on while this page
 * watches for its end, or an API key. Whether it is connected decides
 * whether the assistant has a voice, so `me` is asked again when it changes. */
@Composable
private fun XaiConnection(model: ChatModel, now: Connections, load: () -> Unit) {
    val p = Ink
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var keying by remember { mutableStateOf(false) }
    var key by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var said by remember { mutableStateOf<String?>(null) }
    var dropping by remember { mutableStateOf(false) }
    val signing = now.xaiSigning
    val connected = now.xai != null
    LaunchedEffect(signing != null) {
        while (signing != null) {
            delay(3000)
            load()
        }
    }
    var was by remember { mutableStateOf(connected) }
    LaunchedEffect(connected) {
        if (was != connected) model.refresh()
        was = connected
    }
    // A request to the host, its refusal shown here in its own words.
    val ask = { done: () -> Unit, f: (su.libri.core.Host) -> Unit ->
        said = null
        busy = true
        scope.launch {
            val h = model.host
            val r = runCatching { withContext(Dispatchers.IO) { h?.let(f) } }
            busy = false
            r.onSuccess { done(); load() }.onFailure { said = it.message ?: "That failed." }
        }
        Unit
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text("xAI", style = type(TypeScale.body, 1.45f), color = p.ink)
            Text(now.xai ?: "Read aloud, calls, X search and pictures", style = Type.note, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (connected) {
            Quiet("Disconnect", p.mid) { dropping = true }
        } else if (signing == null) {
            Quiet("Sign in with Grok", p.mid) { if (!busy) ask({}) { it.xaiStart() } }
        }
    }
    if (signing != null && !connected) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("Approve on xAI's page if the code matches.", style = Type.note, color = p.mid)
            Text(signing.code, style = type(TypeScale.title, 1.4f, family = Faces.mono, spacing = 0.1.em), color = p.ink, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
            Text(
                "Open xAI's page", style = type(TypeScale.body, 1.45f), color = p.ink, textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).clickable {
                    context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(signing.url)))
                }.padding(vertical = 10.dp),
            )
        }
    }
    if (!connected && signing == null) {
        Column(Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("Needs SuperGrok or X Premium+, or an API key from console.x.ai.", style = Type.note, color = p.mid)
            if (keying) {
                Field(key, { key = it }, "xai-…", lines = 1, secret = true)
                Text(
                    "Use this key", style = type(TypeScale.body, 1.45f), color = if (key.isBlank() || busy) p.soft else p.ink, textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).clickable(enabled = key.isNotBlank() && !busy) {
                        val k = key.trim()
                        ask({ key = ""; keying = false }) { it.xaiKey(k) }
                    }.padding(vertical = 10.dp),
                )
            } else {
                Text(
                    "Use an API key", style = Type.note.copy(textDecoration = TextDecoration.Underline), color = p.mid,
                    modifier = Modifier.clickable { keying = true },
                )
            }
        }
    }
    (said ?: now.xaiSaid)?.let { Text(it, style = Type.note, color = p.mid, modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp)) }
    if (dropping) {
        Sheet({ dropping = false }) {
            Text("Disconnect xAI?", style = type(TypeScale.prose, 1.3f, FontWeight.SemiBold), color = p.ink)
            Text("Read aloud, calls, X search and pictures stop.", style = Type.small, color = p.mid, modifier = Modifier.padding(top = 4.dp, bottom = 14.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Quiet("Cancel", p.mid) { dropping = false }
                Spacer(Modifier.width(6.dp))
                Pressable("Disconnect", filled = true) {
                    dropping = false
                    ask({}) { it.xaiDrop() }
                }
            }
        }
    }
}

/** Voice (App.tsx): the voice read aloud in, with a sample to hear, kept on
 * the host; how fast, kept on this device. */
@Composable
private fun VoicePage(model: ChatModel) {
    val p = Ink
    var voices by remember { mutableStateOf<List<Voice>>(emptyList()) }
    var hearing by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { voices = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.voices() } }.getOrNull() }.orEmpty() }
    androidx.compose.runtime.DisposableEffect(Unit) { onDispose { hush(model) } }
    val picked = model.voice ?: model.me?.voice ?: return DictationSettings(model)
    val nameOf = { id: String -> voices.find { it.id == id }?.name ?: id }
    // The shown voice stays listed while the list loads, or if it fails.
    val all = (voices.map { it.id } + picked).distinct()
    Head("Voice", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    Box {
        var open by remember { mutableStateOf(false) }
        SettingRow("Voice", chevron = false, onClick = { open = true }) {
            Text(nameOf(picked).replaceFirstChar { it.uppercase() }, style = Type.note, color = p.mid)
            Glyph(Glyph.chevronDown, 13.dp, p.soft, Modifier.padding(start = 4.dp))
            Round(
                onClick = {
                    if (hearing) { hush(model); hearing = false } else {
                        hearing = true
                        sample(model, "Hi, I'm ${model.me?.name}. This is how I sound.", picked) { hearing = false }
                    }
                },
                size = 30.dp, modifier = Modifier.padding(start = 6.dp),
            ) { Glyph(if (hearing) Glyph.stop else Glyph.play, 15.dp, p.mid, label = if (hearing) "Stop the sample" else "Hear this voice") }
        }
        PaperMenu(open, { open = false }, modifier = Modifier.heightIn(max = 420.dp)) {
            all.forEach { id -> MenuRow(nameOf(id).replaceFirstChar { it.uppercase() }, picked = id == picked, onClick = { open = false; hush(model); hearing = false; model.pickVoice(id) }) }
        }
    }
    Column(Modifier.padding(horizontal = 20.dp, vertical = 10.dp)) {
        Row {
            Text("Voice speed", style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.weight(1f))
            Text("${"%.1f".format(model.voiceSpeed)}×", style = Type.note, color = p.mid)
        }
        androidx.compose.material3.Slider(
            value = model.voiceSpeed, onValueChange = model::pickVoiceSpeed, valueRange = 0.5f..3f, steps = 24,
            colors = androidx.compose.material3.SliderDefaults.colors(thumbColor = p.ink, activeTrackColor = p.ink, inactiveTrackColor = p.hair, activeTickColor = Color.Transparent, inactiveTickColor = Color.Transparent),
        )
    }
    // How a call runs: the chat answers, spoken, or Grok Voice talks in
    // front and hands requests to the chat.
    SettingRow(
        "Calls",
        if (model.callMode == "grok") "Grok Voice in front" else "Talk with the chat",
        onClick = { model.pickCallMode(if (model.callMode == "grok") "chat" else "grok") },
        chevron = false,
    )
    DictationSettings(model)
}

/** Where dictation is written down: by the host, with its own model or a
 * service (stt.rs), or by this phone's recognizer. The service's address
 * and key are set on the web page. */
@Composable
private fun DictationSettings(model: ChatModel) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var speech by remember { mutableStateOf<Speech?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    val load: suspend () -> Unit = { speech = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.speech() } }.getOrNull() } }
    LaunchedEffect(Unit) { load() }
    val fetching = speech?.models.orEmpty().any { it.downloading != null }
    LaunchedEffect(fetching) {
        while (fetching) {
            delay(1500)
            load()
        }
    }
    val pick = { provider: String, m: String? ->
        said = null
        scope.launch {
            val s = speech ?: return@launch
            runCatching { withContext(Dispatchers.IO) { model.host?.pickSpeech(provider, m ?: s.choice.model) } }
                .onFailure { said = it.message ?: "The host didn't answer." }
            load()
        }
    }
    val names = mapOf("local" to "This host", "xai" to "xAI (Grok)", "openai" to "OpenAI-compatible API")
    val s = speech
    val now = if (model.dictateOnPhone) "This phone" else s?.let { names[it.choice.provider] } ?: "This host"
    Head("Dictation", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    Box {
        var open by remember { mutableStateOf(false) }
        SettingRow("Transcribe with", chevron = false, onClick = { open = true }) {
            Text(now, style = Type.note, color = p.mid)
            Glyph(Glyph.chevronDown, 13.dp, p.soft, Modifier.padding(start = 4.dp))
        }
        PaperMenu(open, { open = false }) {
            s?.let { h ->
                listOfNotNull("local".takeIf { h.local }, "xai", "openai").forEach { id ->
                    MenuRow(names.getValue(id), picked = !model.dictateOnPhone && h.choice.provider == id, onClick = {
                        open = false
                        model.pickDictateOnPhone(false)
                        pick(id, null)
                    })
                }
            }
            MenuRow("This phone", picked = model.dictateOnPhone, onClick = { open = false; model.pickDictateOnPhone(true) })
        }
    }
    if (!model.dictateOnPhone && s?.choice?.provider == "local") s.models.forEach { m ->
        val state = m.downloading?.let { d -> "Fetching ${if (d.total > 0u) (100u * d.got / d.total).toString() else "0"}%" }
            ?: if (m.installed) "Here" else "${m.bytes / 1_000_000u} MB"
        Row(
            Modifier.fillMaxWidth().clickable { pick("local", m.id) }.padding(horizontal = 20.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text((if (m.id == s.choice.model) "● " else "○ ") + m.name, style = type(TypeScale.body, 1.45f), color = p.ink)
                Text(m.about, style = Type.note, color = p.mid)
            }
            Text(state, style = Type.note, color = p.mid, modifier = Modifier.padding(start = 12.dp))
        }
    }
    val note = Modifier.padding(horizontal = 20.dp, vertical = 4.dp)
    when {
        model.dictateOnPhone -> Text("Uses this phone's own recognizer where it can.", style = Type.note, color = p.mid, modifier = note)
        s?.choice?.provider == "xai" && !s.xai -> Text("Connect xAI under Connections first.", style = Type.note, color = p.mid, modifier = note)
        s?.choice?.provider == "openai" -> Text("Set the address, model and key on the web.", style = Type.note, color = p.mid, modifier = note)
    }
    said?.let { Text(it, style = Type.note, color = p.accent, modifier = note) }
}

@Composable
private fun Appearance(model: ChatModel) {
    val p = Ink
    Head("Display", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    val next = mapOf("system" to "light", "light" to "dark", "dark" to "system")
    SettingRow("Theme", when (model.theme) { "light" -> "Light"; "dark" -> "Dark"; else -> "As the phone" }, onClick = { model.pickTheme(next[model.theme] ?: "system") }, chevron = false)
    SettingRow("Text size", chevron = false) {
        Round(onClick = { model.pickTextPx(model.textPx - 1) }, enabled = model.textPx > ChatModel.TEXT_MIN, edge = p.frame, size = 28.dp) {
            Text("−", style = Type.small, color = p.ink)
        }
        Text("${model.textPx}px", style = Type.note, color = p.soft, modifier = Modifier.width(48.dp), textAlign = TextAlign.Center)
        Round(onClick = { model.pickTextPx(model.textPx + 1) }, enabled = model.textPx < ChatModel.TEXT_MAX, edge = p.frame, size = 28.dp) {
            Text("+", style = Type.small, color = p.ink)
        }
    }
    Head("Workspaces", Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp))
    SettingRow("Undo after closing one", "${UNDO_CLOSE_MS / 1000} seconds", chevron = false)
    SettingRow("Recently closed keeps it", "$KEPT_CLOSED_DAYS days", chevron = false)
}

@Composable
private fun PaperSwitch(on: Boolean, change: (Boolean) -> Unit) = PaperSwitch(on, true, change)

@Composable
private fun PaperSwitch(on: Boolean, enabled: Boolean, change: (Boolean) -> Unit) {
    val p = Ink
    Switch(
        on, change, enabled = enabled,
        colors = SwitchDefaults.colors(
            checkedThumbColor = p.paper, checkedTrackColor = p.ink,
            uncheckedThumbColor = p.soft, uncheckedTrackColor = p.lift, uncheckedBorderColor = p.frame,
        ),
    )
}

/** Each subscription's windows as bars with a pace mark, what model calls
 * cost at API prices over a span cut the way the person picks, and the
 * switch for the rings (App.tsx UsagePage). */
@Composable
private fun UsagePage(model: ChatModel) {
    val u = model.usage
    Head("Limits", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    if (u == null) Small("Asking…", Modifier.padding(horizontal = 20.dp))
    else if (u.accounts.isEmpty()) Small("No subscription signed in here reports limits.", Modifier.padding(horizontal = 20.dp))
    u?.accounts?.forEach { a -> LimitAccount(a) }
    SpendPart(model)
    if (u != null) {
        Head("Composer", Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp))
        SettingRow("Show usage in the input", chevron = false) { PaperSwitch(u.shown, model::showUsage) }
        Small("Rings by Send show the picked model's limits.", Modifier.padding(horizontal = 20.dp))
    }
    CompactPart(model)
}

/** The points a session may be compacted at, as the page offers them
 * (App.tsx COMPACT_POINTS). */
private val COMPACT_POINTS = listOf(100_000UL, 200_000UL, 300_000UL, 500_000UL, 1_000_000UL)

private fun tokensLabel(n: ULong) = if (n >= 1_000_000UL) "${n / 1_000_000UL}M" else "${n / 1000UL}k"

/** How large a chat's session grows before its harness compacts it
 * (App.tsx CompactSection). */
@Composable
private fun CompactPart(model: ChatModel) {
    val h = model.host ?: return
    var now by remember { mutableStateOf<su.libri.core.CompactPoint?>(null) }
    var failed by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    LaunchedEffect(h) { now = withContext(Dispatchers.IO) { runCatching { h.compact() }.getOrNull() } }
    val n = now ?: return
    val points = (COMPACT_POINTS + n.tokens).distinct().sorted()
    fun pick(tokens: ULong?) = scope.launch {
        failed = ""
        withContext(Dispatchers.IO) { runCatching { h.pickCompact(tokens); h.compact() } }
            .onSuccess { now = it }.onFailure { failed = it.message ?: "Couldn't reach the computer." }
    }
    Head("Sessions", Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp))
    Small("Compact sessions at", Modifier.padding(horizontal = 20.dp))
    Choices(points.map { it.toString() to "${tokensLabel(it)} tokens" }, n.tokens.toString()) { pick(it.toULong()) }
    if (n.picked) SettingRow("Back to the default", chevron = false, onClick = { pick(null) })
    Small("Smaller costs less each turn; the memory keeps everything.", Modifier.padding(horizontal = 20.dp))
    if (failed.isNotEmpty()) Small(failed, Modifier.padding(horizontal = 20.dp))
}

@Composable
private fun LimitAccount(a: su.libri.core.Account) {
    val p = Ink
    val clock = remember(a.readAt) { java.time.format.DateTimeFormatter.ofPattern("h:mm a").format(java.time.Instant.ofEpochSecond(a.readAt).atZone(java.time.ZoneId.systemDefault())) }
    Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 12.dp), verticalAlignment = Alignment.Bottom) {
        Text(a.name, style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.weight(1f))
        Text(listOfNotNull(a.plan, if (a.stale) "as of $clock" else null).joinToString(" · "), style = Type.note, color = p.mid)
    }
    a.windows.forEach { w ->
        val pct = w.pct.toFloat().coerceIn(0f, 100f)
        val now = System.currentTimeMillis() / 1000.0
        val through = w.resetsAt?.let { at -> w.hours?.let { h -> (1 - (at - now) / (h * 3600)).coerceIn(0.0, 1.0).toFloat() } }
        // Five points either side of the mark still reads as on pace.
        val pace = through?.let { if (pct > it * 100 + 5) " · ahead of pace" else if (pct < it * 100 - 5) " · under pace" else " · on pace" } ?: ""
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.Bottom) {
                Text(w.name, style = Type.note, color = p.ink, modifier = Modifier.weight(1f))
                Text("${w.pct.toInt()}%", style = Type.note, color = p.ink)
            }
            Box(Modifier.padding(top = 6.dp).fillMaxWidth().height(10.dp), contentAlignment = Alignment.CenterStart) {
                Box(Modifier.fillMaxWidth().height(6.dp).clip(CircleShape).background(p.hair)) {
                    Box(Modifier.fillMaxWidth(pct / 100f).fillMaxHeight().clip(CircleShape).background(if (pct > 90) p.accent else if (pct >= 70) p.ink else p.mid))
                }
                through?.let { t ->
                    Box(Modifier.fillMaxWidth(t.coerceAtLeast(0.005f)), contentAlignment = Alignment.CenterEnd) {
                        Box(Modifier.size(2.dp, 10.dp).clip(CircleShape).background(p.mid))
                    }
                }
            }
            Text("resets ${w.resets}$pace", style = Type.label, color = p.mid, modifier = Modifier.padding(top = 4.dp))
        }
    }
    a.notes.forEach { Small(it, Modifier.padding(horizontal = 20.dp, vertical = 2.dp)) }
}

private fun dollars(d: Double) = if (d > 0 && d < 0.01) "<$0.01" else "$" + String.format(java.util.Locale.getDefault(), "%,.2f", d)
private fun tokenCount(n: ULong): String {
    val v = n.toDouble()
    return when {
        v >= 1e9 -> String.format(java.util.Locale.getDefault(), "%.1fB", v / 1e9)
        v >= 1e6 -> String.format(java.util.Locale.getDefault(), "%.1fM", v / 1e6)
        v >= 1e3 -> "${(v / 1e3).toLong()}k"
        else -> n.toString()
    }
}
private fun sent(s: su.libri.core.SpendSum) = s.input + s.cacheRead + s.cacheWrite

/** A row of choices, one picked: the page's Choices. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Choices(all: List<Pair<String, String>>, now: String, pick: (String) -> Unit) {
    val p = Ink
    FlowRow(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        all.forEach { (k, name) ->
            val on = k == now
            Box(
                Modifier.heightIn(min = 36.dp).clip(CircleShape).background(if (on) p.ink else androidx.compose.ui.graphics.Color.Transparent)
                    .clickable(role = androidx.compose.ui.semantics.Role.RadioButton) { pick(k) }.padding(horizontal = 14.dp),
                contentAlignment = Alignment.Center,
            ) { Text(name, style = Type.note, color = if (on) p.paper else p.mid) }
        }
    }
}

/** What model calls cost at API prices over a span, from the host's own
 * ledger of every call (App.tsx Spend). */
@Composable
private fun SpendPart(model: ChatModel) {
    val p = Ink
    var span by remember { mutableStateOf("week") }
    var cut by remember { mutableStateOf("caller") }
    var all by remember { mutableStateOf(false) }
    var r by remember { mutableStateOf<su.libri.core.Spend?>(null) }
    var failed by remember { mutableStateOf(false) }
    LaunchedEffect(span) {
        failed = false
        val h = model.host ?: return@LaunchedEffect
        withContext(Dispatchers.IO) { runCatching { h.spend(span) } }.onSuccess { r = it }.onFailure { failed = true }
    }
    Head("Spend", Modifier.padding(start = 20.dp, top = 20.dp, bottom = 4.dp))
    Choices(listOf("day" to "Today", "week" to "7 days", "month" to "30 days"), span) { span = it }
    if (failed) Small("Couldn't read the ledger.", Modifier.padding(horizontal = 20.dp))
    val s = r ?: return
    Column(Modifier.padding(horizontal = 20.dp, vertical = 6.dp)) {
        Text(dollars(s.total.cost), style = type(TypeScale.heading, 1.25f), color = p.ink)
        Text(
            "at API prices · ${s.total.calls} calls · ${tokenCount(sent(s.total))} in, ${tokenCount(s.total.output)} out" +
                if (s.total.unpriced > 0u) " · ${s.total.unpriced} on unpriced models" else "",
            style = Type.note, color = p.mid,
        )
    }
    if (span != "day" && s.days.size > 1) {
        val peak = s.days.maxOf { it.cost }
        Row(Modifier.fillMaxWidth().height(40.dp).padding(horizontal = 20.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.Bottom) {
            s.days.forEach { d ->
                val f = if (peak > 0) (d.cost / peak).toFloat().coerceAtLeast(0.06f) else 0.06f
                Box(Modifier.weight(1f).fillMaxHeight(f).clip(RoundedCornerShape(2.dp)).background(p.frame))
            }
        }
    }
    Choices(listOf("caller" to "Who", "model" to "Model", "company" to "Maker", "harness" to "Harness", "paid" to "Paid by"), cut) { cut = it; all = false }
    val rows = when (cut) {
        "model" -> s.byModel
        "company" -> s.byCompany
        "harness" -> s.byHarness
        "paid" -> s.byPaid
        else -> s.byCaller
    }
    if (rows.isEmpty()) Small("No model calls in this span.", Modifier.padding(horizontal = 20.dp))
    val top = rows.maxOfOrNull { it.cost } ?: 0.0
    (if (all) rows else rows.take(10)).forEach { g ->
        Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.Bottom) {
                Text(g.name, style = type(TypeScale.body, 1.45f), color = p.ink, maxLines = 1, modifier = Modifier.weight(1f))
                Text(if (g.unpriced == g.calls) "no price" else dollars(g.cost), style = type(TypeScale.body, 1.45f), color = p.ink)
            }
            Box(Modifier.padding(top = 4.dp).fillMaxWidth().height(4.dp).clip(CircleShape).background(p.hair)) {
                Box(Modifier.fillMaxWidth(if (top > 0) (g.cost / top).toFloat() else 0f).fillMaxHeight().clip(CircleShape).background(p.frame))
            }
            val share = if (s.total.cost > 0) "${Math.round(g.cost / s.total.cost * 100)}% · " else ""
            Text("$share${g.calls} calls · ${tokenCount(sent(g))} in (${tokenCount(g.cacheRead)} cached) · ${tokenCount(g.output)} out", style = Type.label, color = p.mid, modifier = Modifier.padding(top = 4.dp))
        }
    }
    if (!all && rows.size > 10) {
        Text("Show all ${rows.size}", style = Type.note, color = p.mid, modifier = Modifier.clickable { all = true }.padding(horizontal = 20.dp, vertical = 10.dp))
    }
    Small("Prices checked ${s.pricesChecked}.", Modifier.padding(start = 20.dp, end = 20.dp, top = 8.dp))
}

@Composable
private fun ChatSettings(model: ChatModel, pins: Pins) {
    Head("Chat detail", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    listOf("simple" to "Simple", "standard" to "Standard", "full" to "Full").forEach { (value, title) ->
        SettingRow(title, chevron = false, onClick = { model.pickChatDetail(value) }) {
            if (model.chatDetail == value) Glyph(Glyph.check, 16.dp, Ink.ink)
        }
    }
    Small("Simple shows replies only; Full adds tools and thinking.", Modifier.padding(horizontal = 20.dp))
    Head("Messages", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    SettingRow("Pinned", chevron = false) { PaperSwitch(pins.on.value, pins::turn) }
    Small("Pin messages to find them later.", Modifier.padding(horizontal = 20.dp))
}

@Composable
private fun Notifications(model: ChatModel) {
    val context = LocalContext.current
    val activity = context as? android.app.Activity ?: return
    var on by remember { mutableStateOf(Bell.on(context)) }
    var key by remember { mutableStateOf<String?>(null) }
    var asked by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        key = model.host?.let { runCatching { withContext(Dispatchers.IO) { it.pushKey() } }.getOrNull() }
        asked = true
    }
    val distributors = remember { Bell.distributors(context) }
    Head("This device", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    when {
        !asked -> SettingRow("Notifications", "Asking…", chevron = false)
        key == null -> SettingRow("Notifications", "Not on this host", chevron = false)
        distributors.isEmpty() -> SettingRow("Notifications", "Needs ntfy", chevron = false)
        else -> SettingRow("Notifications", chevron = false) {
            PaperSwitch(on) { want -> if (want) Bell.turnOn(activity, key!!) { on = it } else { Bell.turnOff(context); on = false } }
        }
    }
    Small(
        if (asked && key != null && distributors.isEmpty()) {
            "Install ntfy, open it once, then come back."
        } else {
            "Sent through ntfy, encrypted."
        },
        Modifier.padding(horizontal = 20.dp),
    )
}

@Composable
private fun Memory(model: ChatModel) {
    var window by remember { mutableStateOf<ULong?>(null) }
    LaunchedEffect(Unit) { window = model.host?.let { runCatching { withContext(Dispatchers.IO) { it.forgetWindow() } }.getOrNull() } }
    model.memory?.let { m ->
        Head("Status", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
        Fact("State", MEMORY_WORD[m.state] ?: m.state)
        if (m.state == "failing") {
            m.reason?.let { Text(it, style = type(TypeScale.body, 1.45f), color = Ink.accent, modifier = Modifier.padding(horizontal = 20.dp, vertical = 10.dp)) }
            m.since?.let { Fact("Since", stamp(it)) }
        }
        m.newestSummary?.let { Fact("Newest summary", stamp(it)) }
        m.waiting?.takeIf { it > 0u }?.let { Fact("Waiting", "%,d".format(it.toLong())) }
        m.lines?.let { Fact("Total lines", "%,d".format(it.toLong())) }
        m.bytes?.let { Fact("Total size", sized(it.toLong())) }
    }
    Head("Memory", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    model.me?.forgets.orEmpty().forEach { m ->
        SettingRow("Forget after ${lasted(m.toLong() * 60)}", chevron = false, onClick = { window = m; model.act { model.host?.pickForget(m) } }) {
            if (window == m) Glyph(Glyph.check, 16.dp, Ink.ink)
        }
    }
}

/** A size as a storage page shows one, counted in thousands as memory.tsx
 * `sized` does. */
private fun sized(bytes: Long): String {
    val units = listOf("bytes", "KB", "MB", "GB", "TB")
    var n = bytes.toDouble()
    var unit = 0
    while (n >= 1000 && unit < units.lastIndex) { n /= 1000; unit++ }
    return "${if (unit > 0 && n < 100) "%,.1f".format(n).removeSuffix(".0") else "%,.0f".format(n)} ${units[unit]}"
}

/** Settings' Hosts (the side panel's host and Settings' Machines until
 * 11-06, when they were moved them here): the one in use with its latency, its
 * facts, then every other host this phone knows and the way to add one.
 * Each is asked every 15 seconds while the page shows. */
@Composable
private fun Hosts(model: ChatModel) {
    val p = Ink
    var seen by remember { mutableStateOf<Long?>(null) }
    var down by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        while (true) {
            val info = model.host?.let { runCatching { withContext(Dispatchers.IO) { it.info() } }.getOrNull() }
            down = info == null
            if (info != null) seen = info.millis.toLong()
            model.probeHosts()
            delay(15_000)
        }
    }
    val info = model.info
    Head("In use", Modifier.padding(start = 20.dp, top = 8.dp, bottom = 4.dp))
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(8.dp).background(if (down) p.accent else Active, CircleShape))
        Text(info?.name ?: (if (down) "Not answering" else "Asking…"), style = type(TypeScale.body, 1.45f), color = p.ink, modifier = Modifier.weight(1f).padding(start = 10.dp))
        Text(if (down) "Down" else seen?.let { "Up · $it ms" } ?: "…", style = Type.note, color = p.soft)
    }
    info?.let {
        Fact("Assistant", it.assistant)
        Fact("Version", it.version)
        Fact("Running for", lasted(it.uptime.toLong()))
        Fact("Host ID", it.id.take(8))
        (it.url ?: model.address)?.let { a -> Fact("Address", a.removePrefix("https://").removePrefix("http://")) }
    }
    Head("Other hosts", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    OtherHosts(model)
    SettingRow("Add a host", "By address", onClick = model::addHost)
    Peers(model)
}

/** Peers (hosts.tsx Peers): other people's assistants this one is paired
 * with, to message each other, and the requests either way. */
@Composable
private fun Peers(model: ChatModel) {
    val p = Ink
    val scope = rememberCoroutineScope()
    var peers by remember { mutableStateOf<List<su.libri.core.Peer>?>(null) }
    var said by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    var address by remember { mutableStateOf("") }
    var revoking by remember { mutableStateOf<su.libri.core.Peer?>(null) }
    val read = model.readOnly
    val load = {
        scope.launch {
            model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.peers() } }.onSuccess { peers = it } }
        }
    }
    LaunchedEffect(Unit) { load() }
    val act = { done: String?, call: () -> Unit ->
        said = null
        scope.launch {
            runCatching { withContext(Dispatchers.IO) { call() } }
                .onSuccess { if (done != null) said = done }
                .onFailure { said = it.message ?: "That failed." }
            load()
        }
    }
    val list = peers ?: return
    Head("Peers", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    if (list.isEmpty()) Small("None yet. Pair with another assistant to message each other.", Modifier.padding(horizontal = 20.dp))
    list.forEach { peer ->
        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 20.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(peer.name.ifEmpty { peer.url.removePrefix("https://").removePrefix("http://") }, style = type(TypeScale.body, 1.45f), color = p.ink)
                val status = when (peer.state) {
                    "asking" -> "Waiting for them to approve"
                    "asked" -> "Asks to pair"
                    else -> peer.last?.let {
                        "Last message " + java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM, java.text.DateFormat.SHORT).format(java.util.Date(it.toLong() * 1000))
                    } ?: "Paired"
                }
                Text(status, style = Type.note, color = p.mid)
            }
            if (!read && peer.state == "asked") Quiet("Approve", p.ink) { act(null) { model.host!!.approvePeer(peer.url) } }
            if (!read) Quiet(when (peer.state) { "asked" -> "Decline"; "asking" -> "Withdraw"; else -> "Revoke" }, p.mid) { revoking = peer }
        }
    }
    if (!read) {
        if (adding) {
            Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Field(address, { address = it }, "Address, as http://127.0.0.1:8952", lines = 1)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Pressable("Ask", filled = true, enabled = address.isNotBlank()) {
                        val url = address.trim()
                        adding = false
                        address = ""
                        act("Asked. It pairs once they approve.") { model.host!!.askPeer(url) }
                    }
                    Quiet("Cancel", p.mid) { adding = false }
                }
            }
        } else {
            SettingRow("Pair with an assistant", "By address", onClick = { adding = true })
        }
    }
    said?.let { Small(it, Modifier.padding(horizontal = 20.dp), color = p.ink) }
    revoking?.let { peer ->
        val name = peer.name.ifEmpty { "this assistant" }
        ConfirmDialog(
            title = when (peer.state) { "asked" -> "Decline $name's request?"; "asking" -> "Withdraw this request?"; else -> "Unpair $name?" },
            body = if (peer.state == "paired") "You can't message each other until you pair again." else null,
            confirm = when (peer.state) { "asked" -> "Decline"; "asking" -> "Withdraw"; else -> "Unpair" },
            danger = true,
            onDismiss = { revoking = null },
            onConfirm = { revoking = null; act(null) { model.host!!.revokePeer(peer.url) } },
        )
    }
}

/** A Temp tab's key for its Temp (ChatModel.temp): its id, or "" for the
 * host's current Temp and for one still being made. */
internal fun tempKey(arg: String?): String = if (arg == null || arg == "new") "" else arg
