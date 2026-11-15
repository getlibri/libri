package su.libri

import android.annotation.SuppressLint
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import coil3.compose.AsyncImage
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.delay
import su.libri.core.LibraryPage
import su.libri.core.WatchItem

/** What the Library has open (library.tsx Shelved). */
private sealed interface Shelved {
    data class Page(val name: String) : Shelved
    data class Listed(val name: String) : Shelved
}

private fun LibraryPage.shown() = title?.takeIf { it.isNotBlank() } ?: name

/** When something changed, in a list's few words (library.tsx when). */
private fun whenWords(secs: ULong): String {
    val d = Date(secs.toLong() * 1000)
    val today = DateFormat.getDateInstance().format(d) == DateFormat.getDateInstance().format(Date())
    return if (today) DateFormat.getTimeInstance(DateFormat.SHORT).format(d)
    else java.text.SimpleDateFormat("MMM d", java.util.Locale.getDefault()).format(d)
}

/** The words + sends the chat: a request as the person might word it, so the
 * model makes it with whatever tool suits (library.tsx asking). */
fun libraryAsking(what: String) = "Make this for my Library, as a page on my site: $what"

/** The Library tab: the pages show_page published, the latest first, then the
 * book's lists; a page opens in place, and + asks the chat for a new one
 * (library.tsx LibraryShelf). Asked again while in view. */
@Composable
/** `ask` sends the person's words to the chat; `compose` puts a page's
 * message in the chat's composer, for the person to send or not. */
fun LibraryPlace(model: ChatModel, shown: Boolean, compose: (String) -> Unit, ask: (String) -> Unit) {
    val p = Ink
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(owner, shown) {
        if (!shown) return@LaunchedEffect
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.refreshLibrary()
                delay(15_000)
            }
        }
    }
    var open by remember { mutableStateOf<Shelved?>(null) }
    var asking by rememberSaveable { mutableStateOf(false) }
    BackHandler(enabled = shown && open != null) { open = null }
    val library = model.library
    // A page asked open from elsewhere (a link, or one published live).
    val asked = model.opening
    LaunchedEffect(asked) {
        if (asked == null) return@LaunchedEffect
        open = Shelved.Page(asked)
        model.opening = null
        model.refreshLibrary()
    }
    open?.let { s ->
        // An item is a page of its own, kept clear of the bar.
        Box(Modifier.padding(bottom = LocalBarRoom.current)) { ShelvedPage(model, s, compose) { open = null } }
        return
    }
    Box(Modifier.fillMaxSize()) {
        // No head, as on Chat: the bar below already names the place.
        Column(Modifier.fillMaxSize()) {
            // The box sits above the list, as on Tasks, so the floating + opens
            // it in sight however far the list is scrolled.
            // It runs up under the status bar (Places' "library").
            val top = statusTop
            if (asking) Box(Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, top = top + 12.dp)) {
                AskBox({ ask(libraryAsking(it)); asking = false }) { asking = false }
            }
            Gallery(model, library, if (asking) 0.dp else top) { open = it }
        }
        Fab("Ask for something new", open = asking) { asking = !asking }
    }
}

/** How long ago something changed, in a card's few words (library.tsx ago). */
private fun agoWords(secs: ULong): String {
    val gone = maxOf(0L, System.currentTimeMillis() / 1000 - secs.toLong())
    return when {
        gone < 60 -> "just now"
        gone < 3600 -> "${gone / 60} min. ago"
        gone < 86400 -> "${gone / 3600} hr. ago"
        gone < 2 * 86400 -> "yesterday"
        gone < 7 * 86400 -> "${gone / 86400} days ago"
        else -> whenWords(secs)
    }
}

/** The pages grouped by when they changed, newest first (library.tsx grouped). */
private fun grouped(pages: List<LibraryPage>): List<Pair<String, List<LibraryPage>>> {
    val midnight = java.util.Calendar.getInstance().apply {
        set(java.util.Calendar.HOUR_OF_DAY, 0); set(java.util.Calendar.MINUTE, 0); set(java.util.Calendar.SECOND, 0); set(java.util.Calendar.MILLISECOND, 0)
    }.timeInMillis / 1000
    val sorted = pages.sortedByDescending { it.changed }
    return listOf(
        "Today" to sorted.filter { it.changed.toLong() >= midnight },
        "This week" to sorted.filter { it.changed.toLong() in (midnight - 6 * 86400) until midnight },
        "Earlier" to sorted.filter { it.changed.toLong() < midnight - 6 * 86400 },
    ).filter { it.second.isNotEmpty() }
}

/** A Library this long or longer gets a box to find in it (library.tsx FINDABLE). */
private const val FINDABLE = 9

/** The Library as a gallery of cards, two or more to a row (library.tsx
 * LibraryShelf). Each page's card shows its still as an image (Coil, which
 * keeps it by version), never a live page; the page runs only when opened. */
@Composable
private fun Gallery(model: ChatModel, library: su.libri.core.LibraryShelf?, top: Dp, open: (Shelved) -> Unit) {
    val p = Ink
    var find by rememberSaveable { mutableStateOf("") }
    val words = find.trim().lowercase()
    val all = library?.pages.orEmpty()
    val allLists = library?.lists.orEmpty()
    val pages = if (words.isEmpty()) all else all.filter { "${it.shown()} ${it.name}".lowercase().contains(words) }
    val lists = if (words.isEmpty()) allLists else allLists.filter { "${it.name} ${it.items.joinToString(" ")}".lowercase().contains(words) }
    LazyVerticalGrid(
        GridCells.Adaptive(150.dp),
        Modifier.fillMaxSize().edgeFade(Ink.paper, top = if (top > 0.dp) top + TOP_FADE else 0.dp, bottom = LocalBarRoom.current + 24.dp),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = top + 16.dp, bottom = FAB_CLEARANCE + LocalBarRoom.current),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (all.size + allLists.size >= FINDABLE) item(span = { GridItemSpan(maxLineSpan) }, key = "find") { FindBox(find) { find = it } }
        if (library != null && all.isEmpty() && allLists.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) {
            Column(
                Modifier.fillMaxWidth().border(1.dp, p.frame, RoundedCornerShape(16.dp)).padding(horizontal = 24.dp, vertical = 40.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Glyph(Glyph.library, 32.dp, p.soft)
                Text(
                    "Pages the assistant makes for you and the lists it keeps are shelved here. + asks for one.",
                    style = type(TypeScale.control, 1.5f), color = p.mid, textAlign = TextAlign.Center,
                    modifier = Modifier.padding(top = 12.dp),
                )
            }
        }
        if (words.isNotEmpty() && pages.isEmpty() && lists.isEmpty()) item(span = { GridItemSpan(maxLineSpan) }) {
            Text("Nothing in the Library matches.", style = Type.small, color = p.mid, modifier = Modifier.padding(horizontal = 4.dp))
        }
        for ((name, group) in grouped(pages)) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "g/$name") { GroupHead(name, group.size) }
            items(group, key = { "p/${it.name}" }) { pg -> PageCard(model, pg) { open(Shelved.Page(pg.name)) } }
        }
        if (lists.isNotEmpty()) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "g/lists") { GroupHead("Lists", lists.size) }
            items(lists, key = { "l/${it.name}" }) { l -> ListCard(l) { open(Shelved.Listed(l.name)) } }
        }
    }
}

@Composable
private fun GroupHead(name: String, count: Int) {
    Row(Modifier.padding(start = 4.dp, top = 8.dp), verticalAlignment = Alignment.Bottom) {
        Text(name, style = type(TypeScale.label, 1.3f, FontWeight.SemiBold), color = Ink.ink)
        Text("$count", style = Type.label, color = Ink.soft, modifier = Modifier.padding(start = 8.dp))
    }
}

@Composable
private fun FindBox(text: String, change: (String) -> Unit) {
    val p = Ink
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.xl)).background(p.lift).border(1.dp, p.hair, RoundedCornerShape(Radii.xl)).padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Glyph(Glyph.find, 16.dp, p.mid)
        Box(Modifier.weight(1f)) {
            if (text.isEmpty()) Text("Find in the Library", style = Type.body, color = p.soft)
            BasicTextField(
                text, change,
                textStyle = Type.body.copy(color = p.ink),
                cursorBrush = SolidColor(p.ink),
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Find in the Library" },
            )
        }
    }
}

/** A page's own faint hue, from its name, while its still is on its way
 * (library.tsx tint). */
private fun tint(name: String, paper: androidx.compose.ui.graphics.Color): androidx.compose.ui.graphics.Color {
    // FNV-1a, so names a letter apart land far apart on the wheel.
    var h = 0x811c9dc5.toInt()
    for (c in name) h = (h xor c.code) * 0x01000193
    val hue = (h.toUInt() % 360u).toFloat()
    return androidx.compose.ui.graphics.lerp(paper, androidx.compose.ui.graphics.Color.hsl(hue, 0.45f, 0.55f), 0.16f)
}

private val CardShape = RoundedCornerShape(Radii.xl)

@Composable
private fun CardFrame(open: () -> Unit, top: @Composable () -> Unit, title: String, glyph: androidx.compose.ui.graphics.vector.ImageVector, detail: String) {
    val p = Ink
    Column(
        Modifier.fillMaxWidth().clip(CardShape).background(p.lift).border(1.dp, p.hair, CardShape).clickable(onClick = open),
    ) {
        Box(Modifier.fillMaxWidth().aspectRatio(4f / 3f)) { top() }
        Box(Modifier.fillMaxWidth().height(1.dp).background(p.hair))
        Column(Modifier.padding(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 10.dp)) {
            Text(title, style = type(TypeScale.label, 1.35f, FontWeight.Medium), color = p.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Glyph(glyph, 14.dp, p.soft)
                Text(detail, style = Type.label, color = p.soft, maxLines = 1)
            }
        }
    }
}

/** A page's card: its still in a box of fixed shape, tinted until the
 * image arrives, so nothing shifts (library.tsx PageCard). */
@Composable
private fun PageCard(model: ChatModel, pg: LibraryPage, open: () -> Unit) {
    val p = Ink
    CardFrame(open, top = {
        Box(Modifier.fillMaxSize().background(tint(pg.name, p.paper)), contentAlignment = Alignment.Center) {
            Glyph(Glyph.page, 28.dp, p.soft.copy(alpha = 0.6f))
            model.snapshotUrl(pg.name, pg.changed)?.let { at ->
                AsyncImage(
                    model = at,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    alignment = Alignment.TopCenter,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }, title = pg.shown(), glyph = Glyph.page, detail = agoWords(pg.changed))
}

/** A list's card: its first items in place of a picture. */
@Composable
private fun ListCard(l: su.libri.core.LibraryList, open: () -> Unit) {
    val p = Ink
    CardFrame(open, top = {
        Column(Modifier.fillMaxSize().background(p.paper).padding(start = 12.dp, end = 12.dp, top = 12.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            if (l.items.isEmpty()) Text("Empty", style = Type.label, color = p.soft)
            for (item in l.items.take(5)) Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.padding(end = 8.dp).size(4.dp).background(p.soft, CircleShape))
                Text(item, style = Type.label, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }, title = l.name, glyph = Glyph.list, detail = "${l.items.size} ${if (l.items.size == 1) "item" else "items"}")
}

/** + on the Library or Tasks: the person's words, for `send` (library.tsx Ask). */
@Composable
fun AskBox(send: (String) -> Unit, placeholder: String = "A document, a dashboard, a page…", go: String = "Ask", close: () -> Unit) {
    val p = Ink
    var text by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val submit = { if (text.isNotBlank()) send(text.trim()) }
    Row(
        Modifier.fillMaxWidth().card(p, radius = Radii.xl).padding(start = 12.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            if (text.isEmpty()) Text(placeholder, style = Type.body, color = p.soft)
            BasicTextField(
                text, { text = it },
                textStyle = Type.body.copy(color = p.ink),
                cursorBrush = SolidColor(p.ink),
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { submit() }),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )
        }
        Quiet(go, color = if (text.isBlank()) p.soft else p.ink) { submit() }
    }
    BackHandler(onBack = close)
}

/** A host page's address with the app's look: its theme, and its paper, which
 * is flatter than the web's, so a page painted on the web's would show as a
 * pale box (main.tsx reads both and stores neither). */
fun hostLook(at: String, p: Paper): String =
    at + (if (p.dark) "&theme=dark" else "") + "&paper=" + "%06X".format(p.paper.toArgb() and 0xFFFFFF)

/** A page of the host's own (a Library page's `#page-alone=`, a built-in's
 * `#builtin-alone=`) in a WebView, which hosts its MCP App as the web does,
 * so the app carries no MCP Apps host of its own. A message from it goes to
 * `compose`. */
@SuppressLint("SetJavaScriptEnabled", "RequiresFeature")
@Composable
fun HostWebView(at: String, compose: (String) -> Unit, modifier: Modifier = Modifier) {
    val ask = rememberFileAsk()
    AndroidView(
        factory = { c ->
            WebView(c).apply {
                settings.javaScriptEnabled = true
                // The site keeps its theme and hosts in local storage.
                settings.domStorageEnabled = true
                webViewClient = WebViewClient()
                // Files in (a picker for the page's file input) and out
                // (links to download, and files the page saves; WebFiles.kt).
                webChromeClient = pickingChrome(ask)
                downloadsLinks(this)
                // A page's message, by way of the host's page: given only
                // to the host's own origin, never to the page's frames,
                // whose origin is opaque.
                if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
                    val origin = android.net.Uri.parse(at).let { "${it.scheme}://${it.authority}" }
                    WebViewCompat.addWebMessageListener(this, "LibriApp", setOf(origin)) { _, message, _, main, _ ->
                        if (main) message.data?.takeIf { it.isNotBlank() }?.let { if (!savedFromPage(context, it)) compose(it) }
                    }
                }
                loadUrl(at)
            }
        },
        update = { if (it.url != at) it.loadUrl(at) },
        onRelease = { it.destroy() },
        modifier = modifier,
    )
}

/** What the Library opened, whole: a page in a WebView, or a list's items. */
@SuppressLint("SetJavaScriptEnabled", "RequiresFeature")
@Composable
private fun ShelvedPage(model: ChatModel, s: Shelved, compose: (String) -> Unit, close: () -> Unit) {
    val p = Ink
    val library = model.library
    val page = (s as? Shelved.Page)?.let { o -> library?.pages?.firstOrNull { it.name == o.name } }
    val list = (s as? Shelved.Listed)?.let { o -> library?.lists?.firstOrNull { it.name == o.name } }
    // No head on a phone: the way back floats at the
    // top left, as the chat's buttons do, and the bar names the place.
    Box(Modifier.fillMaxSize().background(p.paper)) {
        when (s) {
            is Shelved.Page -> {
                // The WebView's storage starts empty, so the app's theme and
                // paper go in the address (hostLook); the page applies them.
                val at = model.pageUrl(s.name)?.let { hostLook(it, p) }
                // The host's page of this page alone (the site's `#page-alone=`,
                // card.tsx PageAlone), which hosts it as an MCP App as the web
                // does, so the app carries no MCP Apps host of its own.
                if (at != null) HostWebView(at, compose, Modifier.fillMaxSize().statusBarsPadding())
            }
            is Shelved.Listed -> LazyColumn(
                Modifier.fillMaxSize().edgeFade(p.paper, top = statusTop + TOP_FADE),
                contentPadding = PaddingValues(start = 14.dp, end = 14.dp, top = statusTop + CHROME_TOP, bottom = 16.dp),
            ) {
                if (list == null) item { Text(if (library == null) "" else "This is gone from the Library.", style = Type.small, color = p.mid) }
                else {
                    items(list.items) { item ->
                        Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 8.dp), verticalAlignment = Alignment.Top) {
                            Box(Modifier.padding(top = 9.dp, end = 12.dp).size(6.dp).background(p.frame, CircleShape))
                            Text(item, style = Type.body, color = p.ink)
                        }
                    }
                }
            }
        }
        FloatButton(Glyph.back, "Back to the Library", close, Modifier.align(Alignment.TopStart).statusBarsPadding().padding(start = 14.dp, top = 10.dp))
    }
}

/** What a watch is, in a word (library.tsx kindOf). */
private fun kindOf(w: WatchItem) = when {
    w.looks != null -> "Watch"
    w.at == null -> "On push"
    w.kind == "note" -> "Reminder"
    else -> "Routine"
}

/** A watch held: which, and where it stood on screen. */
data class HeldWatch(val id: ULong, val bounds: Rect)

/** The Workers tile's Watches tab: each watch and reminder as a card. */
fun LazyListScope.watchList(model: ChatModel, held: (HeldWatch) -> Unit) {
    items(model.watches, key = { "w/${it.id}" }) { w -> WatchCard(w, held) }
}

/** How often a watch repeats, as a person says it: Daily, Every 30m
 * (library.tsx repeats). */
private fun repeats(every: String?) = when {
    every == null -> "Once"
    every.first().isDigit() -> "Every $every"
    else -> every.replaceFirstChar { it.uppercase() }
}

/** Everything a watch holds, line by line, for its card opened
 * (library.tsx watchDetails). */
private fun watchDetails(w: WatchItem): List<Pair<String, String>> {
    val next = when {
        w.pushed -> "When a push arrives"
        w.at != null -> java.text.SimpleDateFormat("EEE, MMM d, h:mm a", java.util.Locale.getDefault()).format(java.util.Date(w.at!!.toLong() * 1000))
        else -> "Not timed"
    }
    return buildList {
        add((if (w.kind == "ask") "Ask" else "Note") to w.text)
        add("Next" to next)
        add("Repeats" to repeats(w.every))
        w.looks?.let { add("Looks at" to (w.arguments?.let { a -> "$it $a" } ?: it)) }
        if (w.tests.isNotEmpty()) add("Tests" to w.tests.joinToString("\n"))
        if (w.fields.isNotEmpty()) add("Sees" to w.fields.joinToString(", "))
        add("Goes to" to (w.worker?.let { "$it's thread" } ?: "The main chat"))
        if (w.shared.isNotEmpty()) add("Shared with" to w.shared.joinToString(", "))
        add("State" to if (w.active) "On" else "Off")
        w.failing?.let { add("Failing" to it) }
    }
}

/** One watch's card: a tap opens it in place to show all it holds, another
 * closes it; Snooze and Delete on a hold (library.tsx WatchRow). */
@Composable
private fun WatchCard(w: WatchItem, held: (HeldWatch) -> Unit) {
    val p = Ink
    var where by remember { mutableStateOf<LayoutCoordinates?>(null) }
    var open by rememberSaveable(w.id.toLong()) { mutableStateOf(false) }
    val line = type(TypeScale.caption, 15.525f / TypeScale.caption)
    val note = listOfNotNull(
        w.at?.let { wakeWords(it.toLong()) },
        w.every?.let { "every $it" },
        w.worker?.let { "in $it's thread" },
        if (!w.active) "off" else null,
    ).joinToString(" · ")
    Column(
        Modifier.fillMaxWidth().card(p, radius = Radii.xl, framed = false).border(1.dp, p.hair, RoundedCornerShape(Radii.xl))
            .onGloballyPositioned { where = it }
            .combinedClickable(
                onClickLabel = if (open) "Close" else "Open",
                onClick = { open = !open },
                onLongClickLabel = "Actions",
                onLongClick = { where?.let { held(HeldWatch(w.id, it.boundsInWindow())) } },
            )
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(8.dp).then(if (w.failing != null) Modifier.background(p.accent, CircleShape) else if (w.active) Modifier.border(1.5.dp, Active, CircleShape) else Modifier.background(p.frame, CircleShape)))
            Text(kindOf(w), style = line.copy(fontWeight = FontWeight.SemiBold), color = p.ink, maxLines = 1, modifier = Modifier.padding(start = 6.dp))
            if (w.failing != null) Text(
                "Failing", style = line.copy(fontWeight = FontWeight.Medium), color = p.accent, maxLines = 1,
                modifier = Modifier.padding(start = 6.dp).background(p.accent.copy(alpha = 0.1f), RoundedCornerShape(4.dp)).padding(horizontal = 6.dp),
            )
            Text(note, style = line, color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp).weight(1f))
        }
        if (open) Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            val label = type(TypeScale.label, 1.35f)
            for ((name, value) in watchDetails(w)) Row {
                // The same column as the web's (library.tsx WatchRow).
                Text(name, style = line.copy(lineHeight = label.lineHeight), color = p.soft, modifier = Modifier.width(80.dp))
                Text(value, style = label, color = p.ink, modifier = Modifier.padding(start = 12.dp).weight(1f))
            }
        } else Text(w.text, style = type(TypeScale.label, 1.35f, FontWeight.Medium), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
    }
}

/** A held watch's menu: Snooze, which turns to the nap times, and Delete;
 * both edit the book with no model call (library.tsx WatchMenu). */
@Composable
fun WatchMenu(model: ChatModel, h: HeldWatch, close: () -> Unit) {
    val w = model.watches.firstOrNull { it.id == h.id }
    var snoozing by remember(h) { mutableStateOf(false) }
    if (w == null) {
        LaunchedEffect(h) { close() }
        return
    }
    val items = if (snoozing) {
        // On from when it was due, or from now.
        val from = java.util.Calendar.getInstance().apply { timeInMillis = maxOf(System.currentTimeMillis(), (w.at?.toLong() ?: 0) * 1000) }
        napPresets(from).map { n -> Triple(n.label, Glyph.clock) { model.watchSnooze(w.id, n.until); close() } }
    } else buildList {
        if (w.at != null) add(Triple("Snooze", Glyph.clock) { snoozing = true })
        add(Triple("Delete", Glyph.trash) { model.watchDelete(w.id); close() })
    }
    HoldMenu(h.bounds, framed = false, items, close) { WatchCard(w) {} }
}
