package su.libri

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
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
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.text.DateFormat
import java.util.Calendar
import java.util.Date
import su.libri.core.ClientException
import su.libri.core.Found
import su.libri.core.FoundHit
import su.libri.core.FoundLine
import su.libri.core.FoundSource

// Search over everything said, the chat's whole past and every worker's
// log, as the web app's search.tsx: results as typed, grouped by where they
// were said, and a hit opened among the messages around it. The host keeps
// the index, so the app only asks and draws.

/** Who said a line, as the chat names them. */
private fun who(kind: String, name: String) = when (kind) {
    "user" -> "You"
    "talk" -> name
    "tool" -> "Tool"
    "echo" -> "Output"
    "work" -> "Report"
    "note" -> "Note"
    "from" -> "From a worker"
    "to" -> "To a worker"
    else -> kind
}

private fun day(s: Long, clock: Boolean = false): String {
    if (s == 0L) return ""
    val d = Date(s * 1000)
    val now = Calendar.getInstance()
    val then = Calendar.getInstance().apply { time = d }
    val date = if (now.get(Calendar.YEAR) == then.get(Calendar.YEAR)) {
        android.text.format.DateFormat.format("MMM d", d).toString()
    } else {
        DateFormat.getDateInstance(DateFormat.MEDIUM).format(d)
    }
    return if (clock) "$date, ${DateFormat.getTimeInstance(DateFormat.SHORT).format(d)}" else date
}

/** Markdown's emphasis and code marks, which read as noise in plain text. */
private fun plain(t: String) = t.replace("**", "").replace("`", "")

/** The words the index matched in a snippet, between U+0002 and U+0003. */
private fun matched(snippet: String) =
    Regex("\u0002([^\u0003]*)\u0003").findAll(snippet).map { it.groupValues[1].lowercase() }.toSet()

/** A snippet with its matched words marked. */
private fun marked(text: String, p: Paper): AnnotatedString = buildAnnotatedString {
    // A tool call's input is JSON, its line breaks escaped: spaces read better.
    plain(text).replace("\\n", " ").split('\u0002', '\u0003').forEachIndexed { k, part ->
        if (k % 2 == 1) withStyle(SpanStyle(background = p.frame.copy(alpha = 0.6f), color = p.ink, fontWeight = FontWeight.Medium)) { append(part) }
        else append(part)
    }
}

/** A whole line with the matched words marked wherever they stand whole. */
private fun lit(text: String, words: Set<String>, p: Paper): AnnotatedString = buildAnnotatedString {
    if (words.isEmpty()) return@buildAnnotatedString append(text)
    val re = Regex("(?<![\\p{L}\\p{N}])(" + words.joinToString("|") { Regex.escape(it) } + ")(?![\\p{L}\\p{N}])", RegexOption.IGNORE_CASE)
    var at = 0
    for (m in re.findAll(text)) {
        append(text.substring(at, m.range.first))
        withStyle(SpanStyle(background = p.frame.copy(alpha = 0.6f), color = p.ink)) { append(m.value) }
        at = m.range.last + 1
    }
    append(text.substring(at))
}

/** The search page, over the places as the terminal is. Back steps from a
 * hit to the list, then out. */
@Composable
fun SearchScreen(model: ChatModel, close: () -> Unit) {
    val p = Ink
    val host = model.host ?: return
    val name = model.me?.name ?: model.info?.assistant ?: "Libri"
    var q by rememberSaveable { mutableStateOf("") }
    var newest by rememberSaveable { mutableStateOf(false) }
    var found by remember { mutableStateOf<Result<Found>?>(null) }
    var picked by remember { mutableStateOf<FoundHit?>(null) }
    var sources by remember { mutableStateOf(emptyMap<String, FoundSource>()) }
    val field = remember { FocusRequester() }
    val focus = LocalFocusManager.current
    BackHandler { if (picked != null) picked = null else close() }
    // An opened hit takes the field's focus, so the keyboard goes and the
    // first Back steps out of the hit rather than only shutting the keyboard.
    LaunchedEffect(picked) { if (picked != null) focus.clearFocus() }
    LaunchedEffect(Unit) {
        field.requestFocus()
        sources = withContext(Dispatchers.IO) { runCatching { host.sources() }.getOrNull() }.orEmpty().associateBy { it.src }
    }
    // As typed: a newer word cancels this one's asking.
    LaunchedEffect(q, newest) {
        if (q.isBlank()) return@LaunchedEffect run { found = null }
        delay(40)
        found = withContext(Dispatchers.IO) { runCatching { host.find(q, newest) } }
    }
    val label = { src: String -> if (src.isEmpty()) "Chat" else sources[src]?.name ?: src }
    Column(
        Modifier.fillMaxSize().background(p.paper)
            .windowInsetsPadding(WindowInsets.statusBars.union(WindowInsets.navigationBars).union(WindowInsets.ime)),
    ) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Round(onClick = { if (picked != null) picked = null else close() }, size = 40.dp) {
                Glyph(Glyph.chevronLeft, 20.dp, p.mid, label = if (picked != null) "Back to results" else "Close search")
            }
            Glyph(Glyph.find, 18.dp, p.soft, Modifier.padding(start = 4.dp, end = 8.dp))
            BasicTextField(
                value = q,
                onValueChange = { q = it; picked = null },
                singleLine = true,
                textStyle = Type.body.copy(color = p.ink),
                cursorBrush = SolidColor(p.ink),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { }),
                modifier = Modifier.weight(1f).focusRequester(field),
                decorationBox = { inner ->
                    Box {
                        if (q.isEmpty()) Text("Search everything said", style = Type.body, color = p.soft)
                        inner()
                    }
                },
            )
            Row(Modifier.padding(start = 6.dp).clip(CircleShape).border(1.dp, p.hair, CircleShape).padding(2.dp)) {
                listOf(false to "Best", true to "Newest").forEach { (by, word) ->
                    // An opened hit has no order to change: the switch rests.
                    val rests = picked != null
                    Text(
                        word,
                        style = Type.note,
                        color = if (newest == by && !rests) p.ink else p.soft,
                        modifier = Modifier.clip(CircleShape).background(if (newest == by && !rests) p.hover else p.paper)
                            .clickable(enabled = !rests) { newest = by }.padding(horizontal = 10.dp, vertical = 5.dp),
                    )
                }
            }
        }
        val hit = picked
        if (hit != null) {
            Place(model, hit, label(hit.src), name)
            return@Column
        }
        val f = found
        val hits = f?.getOrNull()
        when {
            q.isBlank() -> Hint("Everything said here and by every worker. Words in quotes are found together.")
            f == null -> Unit
            hits == null -> Hint(failure(f.exceptionOrNull()))
            hits.hits.isEmpty() -> Hint("Nothing found.")
            else -> {
                // Grouped by where each was said, a group's place set by its
                // best hit; Newest is one time order, each row named.
                val groups = if (newest) mapOf("*" to hits.hits) else hits.hits.groupBy { it.src }
                LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 16.dp)) {
                    if (hits.loose) item { Hint("No message has every word; these have some.") }
                    groups.forEach { (src, hs) ->
                        if (src != "*") item(key = "head:$src") {
                            val s = sources[src]
                            Row(Modifier.padding(start = 20.dp, end = 20.dp, top = 14.dp, bottom = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Text(label(src), style = Type.note.copy(fontWeight = FontWeight.SemiBold), color = p.mid)
                                if (s != null) Text(listOfNotNull(day(s.began), "archived".takeIf { s.archived }).joinToString(" · "), style = Type.note, color = p.soft)
                            }
                        }
                        items(hs, key = { "${it.src}:${it.at}" }) { h ->
                            Column(
                                Modifier.fillMaxWidth().padding(horizontal = 8.dp).clip(RoundedCornerShape(Radii.xl))
                                    .clickable { picked = h }.padding(horizontal = 12.dp, vertical = 8.dp),
                            ) {
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    if (src == "*") Text(label(h.src), style = Type.label.copy(fontWeight = FontWeight.SemiBold), color = p.mid)
                                    Text(who(h.kind, name), style = Type.label, color = p.soft)
                                    Text(day(h.time), style = Type.label, color = p.soft)
                                }
                                Text(marked(h.text, p), style = Type.small, color = p.mid, maxLines = 3, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            }
        }
    }
}

/** Why a search failed, as the web says it: the host's own words, else the status. */
private fun failure(e: Throwable?): String = when (e) {
    is ClientException.Said -> e.message ?: "Search failed."
    is ClientException.Unreachable -> "Search failed: the host does not answer."
    else -> "Search failed."
}

@Composable
private fun Hint(text: String) {
    Text(text, style = Type.small, color = Ink.soft, modifier = Modifier.padding(horizontal = 20.dp, vertical = 14.dp))
}

/** A hit in its place: the messages around it, more on asking either way. */
@Composable
private fun Place(model: ChatModel, hit: FoundHit, title: String, name: String) {
    val p = Ink
    var before by remember(hit) { mutableStateOf(12uL) }
    var after by remember(hit) { mutableStateOf(12uL) }
    var lines by remember(hit) { mutableStateOf<List<FoundLine>?>(null) }
    var failed by remember(hit) { mutableStateOf(false) }
    val list = rememberLazyListState()
    val density = LocalDensity.current
    var centred by remember(hit) { mutableStateOf(false) }
    val words = remember(hit) { matched(hit.text) }
    LaunchedEffect(hit, before, after) {
        val got = withContext(Dispatchers.IO) { runCatching { model.host?.around(hit.src, hit.at, before, after) }.getOrNull() }
        if (got == null) failed = true else lines = got
    }
    val shown = lines
    if (failed) return Hint("Couldn't open it.")
    if (shown == null) return
    val top = shown.firstOrNull()?.let { it.at > 0uL && it.at + before == hit.at } == true
    val bottom = shown.lastOrNull()?.let { it.at == hit.at + after } == true
    // Centred once, when first drawn; asking for more keeps the reader's place.
    LaunchedEffect(shown) {
        if (!centred) {
            centred = true
            val at = shown.indexOfFirst { it.at == hit.at }.coerceAtLeast(0) + if (top) 1 else 0
            list.scrollToItem(at, -with(density) { 150.dp.roundToPx() })
        }
    }
    Text(
        "$title · ${day(hit.time, clock = true)}",
        style = Type.note,
        color = p.soft,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp),
    )
    LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(12.dp)) {
        if (top) item { More("Earlier") { before += 40uL } }
        // Each run of tool calls and output is one row, as the chat folds
        // them, unless the hit is in it.
        val rows = mutableListOf<List<FoundLine>>()
        for (l in shown) {
            val tool = l.kind == "tool" || l.kind == "echo"
            val last = rows.lastOrNull()
            if (tool && last != null && last.first().kind.let { it == "tool" || it == "echo" }) rows[rows.size - 1] = last + l
            else rows.add(listOf(l))
        }
        val drawn = rows.flatMap { r -> if (r.size > 1 && r.any { it.at == hit.at }) r.map { listOf(it) } else listOf(r) }
        items(drawn, key = { it.first().at.toString() }) { r ->
            val l = r.first()
            val code = l.kind == "tool" || l.kind == "echo"
            if (code && (r.size > 1 || l.at != hit.at)) return@items Run(r)
            val here = l.at == hit.at
            Column(
                Modifier.fillMaxWidth().padding(vertical = 2.dp).clip(RoundedCornerShape(Radii.xl))
                    .then(if (here) Modifier.background(p.hover).border(1.25.dp, p.frame, RoundedCornerShape(Radii.xl)) else Modifier)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            ) {
                Text(who(l.kind, name), style = Type.label.copy(fontWeight = if (l.kind == "user" || l.kind == "talk") FontWeight.SemiBold else FontWeight.Normal), color = if (l.kind == "user" || l.kind == "talk") p.mid else p.soft)
                when {
                    code -> Text(lit(l.text, words, p), style = Type.code, color = p.mid, modifier = Modifier.padding(top = 2.dp))
                    // The hit keeps its matched words marked, in plain text:
                    // markdown takes no marks.
                    here -> Text(lit(plain(l.text), words, p), style = Type.small, color = p.ink, modifier = Modifier.padding(top = 2.dp))
                    else -> Markdowned(l.text + if (l.cut) " …" else "")
                }
            }
        }
        if (bottom) item { More("Later") { after += 40uL } }
    }
}

/** A run of tool calls and their output, one row until opened. */
@Composable
private fun Run(run: List<FoundLine>) {
    val p = Ink
    var open by remember { mutableStateOf(false) }
    val calls = run.count { it.kind == "tool" }.takeIf { it > 0 } ?: run.size
    Column(Modifier.padding(horizontal = 12.dp, vertical = 2.dp)) {
        Row(Modifier.clip(RoundedCornerShape(Radii.lg)).clickable { open = !open }.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Glyph(if (open) Glyph.chevronDown else Glyph.chevronRight, 14.dp, p.soft)
            Text("Ran $calls ${if (calls == 1) "tool" else "tools"}", style = Type.note, color = p.soft)
        }
        if (open) run.forEach { Text(it.text, style = Type.code, color = p.mid, maxLines = 6, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 20.dp, top = 2.dp)) }
    }
}

@Composable
private fun More(word: String, onClick: () -> Unit) {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Text(word, style = Type.note, color = Ink.soft, modifier = Modifier.clip(CircleShape).clickable(onClick = onClick).padding(horizontal = 12.dp, vertical = 6.dp))
    }
}
