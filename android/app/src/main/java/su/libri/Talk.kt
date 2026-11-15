package su.libri

import android.app.DownloadManager
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.Settings
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import coil3.compose.AsyncImage
import com.mikepenz.markdown.coil3.Coil3ImageTransformerImpl
import com.mikepenz.markdown.m3.Markdown
import com.mikepenz.markdown.m3.markdownColor
import com.mikepenz.markdown.m3.markdownTypography
import com.mikepenz.markdown.model.markdownPadding
import com.mikepenz.markdown.model.rememberMarkdownState
import su.libri.core.Message
import su.libri.core.Outbox
import su.libri.core.Quote
import su.libri.core.ToolCall
import su.libri.core.gist
import su.libri.core.offerShown
import su.libri.core.quiet

/** What a message can be asked to do, by the menu a hold opens. */
interface Acts {
    fun picture(name: String): String
    fun view(url: String)
    /** A hold on a message drawn at `bounds`, in the window's coordinates. */
    fun hold(m: Message, bounds: Rect)
    fun resend(m: Message)
    fun putBack(m: Message)
    /** Brings the message a quote names into view, where the talk holds it. */
    fun jump(row: ULong) {}
}

/** One line of the message a reply answers; a tap goes to it, as the
 * page's QuoteLine. */
@Composable
fun QuoteLine(q: Quote, modifier: Modifier = Modifier, jump: (() -> Unit)? = null) {
    val p = Ink
    Row(
        modifier
            .clip(RoundedCornerShape(8.dp))
            .then(if (jump != null) Modifier.clickable(onClick = jump) else Modifier)
            .padding(start = 6.dp, end = 8.dp, top = 3.dp, bottom = 3.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // A straight bar of its own: drawn at the clipped edge it bent into a "(".
        Box(Modifier.padding(end = 6.dp).size(width = 2.dp, height = 14.dp).clip(RoundedCornerShape(1.dp)).background(p.edge))
        Text("Reply to ", style = Type.label, color = p.mid, fontWeight = FontWeight.Medium, maxLines = 1)
        Text(q.text, style = Type.label, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** A hold opens the message's menu under it, as Claude's app does. */
@Composable
fun Modifier.holdable(m: Message, acts: Acts): Modifier {
    var where by remember { mutableStateOf<LayoutCoordinates?>(null) }
    return this
        .onGloballyPositioned { where = it }
        .pointerInput(m.id) {
            detectTapGestures(onLongPress = { where?.let { w -> acts.hold(m, w.boundsInWindow()) } })
        }
}

/** The person's message: their files over the words, framed in --edge with
 * a tail corner, as the page frames what a person says. */
@Composable
fun Mine(m: Message, acts: Acts) {
    val p = Ink
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val most = maxWidth * 0.82f
        Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End) {
            m.files.forEach { name -> Attached(name, acts) }
            m.quote?.let { q -> QuoteLine(q, Modifier.widthIn(max = most).padding(bottom = 4.dp)) { acts.jump(q.id) } }
            if (m.text.isNotBlank()) {
                Box(
                    Modifier
                        .widthIn(max = most)
                        .framed(m.outbox == Outbox.HELD || m.waitsTurn)
                        .holdable(m, acts)
                        .padding(horizontal = 17.dp, vertical = 10.dp),
                ) {
                    Text(m.text, style = Type.body, color = p.ink)
                }
            }
            when {
                m.outbox == Outbox.HELD -> Small("Waiting to send", Modifier.padding(top = 4.dp))
                m.outbox == Outbox.REFUSED -> Row(Modifier.padding(top = 4.dp)) {
                    Small("Not sent · tap to try again", Modifier.clickable { acts.resend(m) }, p.accent)
                    Small(" · ", color = p.accent)
                    Small("put back in the box", Modifier.clickable { acts.putBack(m) }, p.accent)
                }
                m.waitsTurn -> Small("Waiting its turn", Modifier.padding(top = 4.dp))
                else -> m.reaction?.let { Text(it, Modifier.padding(top = 2.dp)) }
            }
        }
    }
}

/** The edge of what a person says: in --edge with a tail corner, dotted while
 * it waits for the host or its turn, as the page draws it. */
@Composable
private fun Modifier.framed(queued: Boolean): Modifier {
    val edge = Ink.mine
    return if (queued) dotted(edge) else border(1.25.dp, edge, RoundedCornerShape(Radii.bubble, Radii.bubble, Radii.tail, Radii.bubble))
}

private fun Modifier.dotted(color: Color) = this.drawWithContent {
    drawContent()
    val r = 20.dp.toPx()
    drawRoundRect(
        color = color,
        cornerRadius = CornerRadius(r),
        style = Stroke(width = 1.25.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(), 3.dp.toPx()))),
    )
}

/** A file the person sent: the photo itself, or its name on a card. */
@Composable
private fun Attached(name: String, acts: Acts) {
    val image = name.substringAfterLast('.', "").lowercase() in IMAGE_KINDS
    if (image) {
        val url = acts.picture(name)
        AsyncImage(
            model = url,
            contentDescription = name,
            contentScale = ContentScale.Fit,
            modifier = Modifier
                .padding(bottom = 6.dp)
                .widthIn(max = 288.dp)
                .heightIn(min = 64.dp, max = 288.dp)
                .clip(RoundedCornerShape(Radii.xl))
                .border(1.dp, Ink.hair, RoundedCornerShape(Radii.xl))
                .clickable { acts.view(url) },
        )
    } else {
        FileCard(name, null)
    }
}

private val IMAGE_KINDS = setOf("jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif")

/** A file's card; with a URL, a tap saves it to Downloads. */
@Composable
fun FileCard(name: String, url: String?) {
    val p = Ink
    val context = LocalContext.current
    Row(
        Modifier
            .padding(bottom = 6.dp)
            .clip(RoundedCornerShape(Radii.xl))
            .border(1.dp, p.hair, RoundedCornerShape(Radii.xl))
            .background(p.lift)
            .then(if (url != null) Modifier.clickable { save(context, url, name) } else Modifier)
            .padding(horizontal = 12.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Glyph(Glyph.file, 18.dp, p.mid)
        Column {
            Text(name, style = Type.small, color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (url != null) Small("Tap to save")
        }
    }
}

private fun save(context: Context, url: String, name: String) {
    val request = DownloadManager.Request(Uri.parse(url))
        .setTitle(name)
        .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
        .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
    context.getSystemService(DownloadManager::class.java).enqueue(request)
}

/** All a message says, as Copy and the rest of the hold menu take it
 * (App.tsx MessageMenu). */
fun Message.words() = text.trim().takeUnless { it.isEmpty() || quiet(it, false) } ?: ""

/** The assistant's message: the calls that led to it as one quiet line, then
 * its words. Words said before a call are the message above (F13). */
@Composable
fun Theirs(m: Message, acts: Acts, files: (String) -> String) {
    val detail = LocalChatDetail.current
    if (detail == "simple" && (m.progress || m.live || m.text.isBlank())) return
    Column(Modifier.fillMaxWidth().holdable(m, acts)) {
        // An offer_replies shown as the replies has no row of its own (offer_shown).
        val shown = m.tools.filterNot { offerShown(it) }
        if (detail != "simple" && shown.isNotEmpty()) {
            Stack(shown.map { Worked.Call(it) }, m.live, thought = m.thinking.isNotBlank())
            CallCards(shown, byPerson = m.id in LocalPersonTurns.current)
        }
        if (detail != "simple" && m.thinking.isNotBlank()) Thought(m.thinking, live = true)
        // A quiet answer never shows, nor its start while it streams (App.tsx).
        if (m.text.isNotBlank() && !quiet(m.text, m.live)) {
            if (m.plain) Reply(m.text, true, files)
            else Said(top = m.tools.isNotEmpty(), bottom = m.picture != null || m.file != null || m.reaction != null) { Reply(m.text, false, files) }
        }
        if (detail == "full") m.metrics?.let { Metrics(it) }
        m.picture?.let { name ->
            val url = files(name)
            AsyncImage(
                model = url,
                contentDescription = name,
                contentScale = ContentScale.Fit,
                modifier = Modifier
                    .padding(top = 4.dp)
                    .widthIn(max = 288.dp)
                    .heightIn(max = 420.dp)
                    .clip(RoundedCornerShape(Radii.xl))
                    .border(1.dp, Ink.hair, RoundedCornerShape(Radii.xl))
                    .clickable { acts.view(url) },
            )
        }
        m.file?.let { FileCard(it, files(it)) }
        m.reaction?.let { Text(it, Modifier.padding(top = 2.dp)) }
    }
}

/** Libri's own word about itself (a restart, its memory going and coming
 * back), as App.tsx `notice` reads it: "(libri) " ahead, or, kept before
 * that, the whole notice in parentheses. */
private val NOTICE = Regex("^\\(libri\\) ([\\s\\S]*)$|^\\(((?:Libri restarted|My memory is) [\\s\\S]*)\\)$")

fun notice(text: String): String? =
    NOTICE.find(text.trim())?.groupValues?.let { g -> g[1].ifEmpty { g[2] } }

/** A notice on a faint wash of the accent, apart from the replies. */
@Composable
fun Notice(text: String) {
    val p = Ink
    Text(
        text,
        style = type(TypeScale.control, 1.5f),
        color = p.mid,
        modifier = Modifier.fillMaxWidth().background(p.accent.copy(alpha = 0.08f), RoundedCornerShape(8.dp)).padding(horizontal = 12.dp, vertical = 8.dp),
    )
}

/** News from outside the talk (a routine, watch, alert, message): one line
 * in --soft with a rule beside it, opening on a tap, as the page folds it. */
@Composable
fun Note(m: Message) {
    val p = Ink
    val detail = LocalChatDetail.current
    var open by remember(detail) { mutableStateOf(detail == "full") }
    val (label, body) = Regex("^\\((\\w+)\\) ([\\s\\S]*)$").find(m.text)?.destructured?.let { (k, b) -> k.replaceFirstChar { it.uppercase() } to b } ?: ("Note" to m.text)
    val opened = remember { Opened() }
    Row(Modifier.fillMaxWidth().clickable { if (open) opened.close() else opened.tap(); open = !open }.animateContentSize().revealed(opened)) {
        Box(Modifier.padding(end = 12.dp).size(2.dp, if (open) 200.dp else 20.dp).background(p.hair))
        Column(Modifier.weight(1f)) {
            Text(
                if (open) label else "$label · ${body.lineSequence().first()}",
                style = type(TypeScale.control, 1.45f, family = Faces.serif).copy(fontStyle = FontStyle.Italic),
                color = p.soft,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (open) Text(body, style = type(TypeScale.control, 1.5f), color = p.soft, modifier = Modifier.padding(top = 4.dp))
        }
    }
}

/** What a run of calls did, in words: "Thought, ran 3 commands, read 2
 * files" (App.tsx `did`). */
private val DID = mapOf(
    "Bash" to Triple("ran", "command", "commands"),
    "BashOutput" to Triple("ran", "command", "commands"),
    "KillShell" to Triple("ran", "command", "commands"),
    "Read" to Triple("read", "file", "files"),
    "Edit" to Triple("edited", "file", "files"),
    "MultiEdit" to Triple("edited", "file", "files"),
    "NotebookEdit" to Triple("edited", "file", "files"),
    "Write" to Triple("wrote", "file", "files"),
    "Grep" to Triple("ran", "search", "searches"),
    "Glob" to Triple("ran", "search", "searches"),
    "ToolSearch" to Triple("ran", "search", "searches"),
    "WebFetch" to Triple("fetched", "page", "pages"),
    "WebSearch" to Triple("searched the web", "time", "times"),
    "Agent" to Triple("sent", "agent", "agents"),
    "Task" to Triple("sent", "agent", "agents"),
    "AskUserQuestion" to Triple("asked", "question", "questions"),
    "TodoWrite" to Triple("updated", "to-do list", "to-do lists"),
    "Retry" to Triple("retried", "model call", "model calls"),
)

fun did(calls: List<ToolCall>, thought: Boolean): String {
    val tally = linkedMapOf<String, Pair<Triple<String, String, String>, Int>>()
    calls.forEach { c ->
        val d = DID[c.name] ?: if (c.name == "Message" || c.name.startsWith("Message to ")) Triple("sent", "message", "messages") else Triple("used", "tool", "tools")
        val key = "${d.first} ${d.third}"
        tally[key] = d to ((tally[key]?.second ?: 0) + 1)
    }
    val said = (if (thought) listOf("thought") else emptyList()) +
        tally.values.map { (d, n) -> "${d.first} $n ${if (n == 1) d.second else d.third}" }
    return said.joinToString(", ").replaceFirstChar { it.uppercase() }
}

/** Claude Code's tools as a person would name them, each with its glyph
 * (App.tsx TOOLS); an MCP tool reads as \"server · tool\". */
private val TOOLS = mapOf(
    "Bash" to ("Shell" to Glyph.shell), "BashOutput" to ("Shell output" to Glyph.shell), "KillShell" to ("Stop shell" to Glyph.shell),
    "Read" to ("Read" to Glyph.file), "Edit" to ("Edit" to Glyph.pen), "MultiEdit" to ("Edit" to Glyph.pen),
    "NotebookEdit" to ("Edit" to Glyph.pen), "Write" to ("Write" to Glyph.pen), "Grep" to ("Search" to Glyph.find),
    "Glob" to ("Find files" to Glyph.find), "WebFetch" to ("Fetch" to Glyph.web), "WebSearch" to ("Web search" to Glyph.web),
    "Agent" to ("Agent" to Glyph.agent), "Task" to ("Agent" to Glyph.agent), "TodoWrite" to ("Todos" to Glyph.list),
    "ToolSearch" to ("Tool search" to Glyph.find), "Skill" to ("Skill" to Glyph.agent), "AskUserQuestion" to ("Question" to Glyph.ask),
    // omp's own: a failed model call it tries again by itself.
    "Retry" to ("Retry" to Glyph.brain),
)

private fun labelOf(name: String) = TOOLS[name]?.first
    ?: Regex("^mcp__(.+?)__(.+)$").find(name)?.destructured?.let { (server, tool) -> "$server · ${tool.replace('_', ' ')}" }
    ?: name

/** Words between workers: the core names these rows, by the worker at the
 * other end, so they miss TOOLS. A call under way is just "Message", until
 * its input has come whole. */
private fun isMessage(name: String) = name == "Message" || Regex("^(Message to|Message from|Reply from) ").containsMatchIn(name)

private fun glyphOf(name: String): ImageVector = TOOLS[name]?.second ?: if (isMessage(name)) Glyph.agent else Glyph.plug

/** What went into a call: nothing yet for a message under way, whose input
 * is still partial JSON. */
private fun said(t: ToolCall) = if (t.name == "Message") "" else t.input

/** What a call was about, as its row names it: the command's first line,
 * a file, a URL. */
private fun about(t: ToolCall) =
    if (isMessage(t.name)) firstLine(said(t))
    else if (t.name == "TodoWrite") todos(t.input)?.let { l -> "${l.count { it.status == "completed" }} of ${l.size} done" } ?: ""
    else gist(said(t)).replace(Regex("^cd\\s+\\S+\\s*&&\\s*"), "")

/** One item of a to-do list, as TodoWrite's input holds it. */
private data class Todo(val content: String, val status: String, val phase: String?)

private fun todos(input: String): List<Todo>? = runCatching {
    val list = org.json.JSONObject(input).getJSONArray("todos")
    (0 until list.length()).map { list.getJSONObject(it) }.map {
        Todo(it.optString("content"), it.optString("status", "pending"), it.optString("phase").ifEmpty { null })
    }
}.getOrNull()

/** A to-do list as a checklist (App.tsx Todos): each item's state by its
 * mark, done ones struck through, under its phase's name. */
@Composable
private fun Todos(list: List<Todo>) {
    val p = Ink
    Column(Modifier.padding(vertical = 2.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        list.forEachIndexed { k, t ->
            if (t.phase != null && t.phase != list.getOrNull(k - 1)?.phase) {
                Text(t.phase, style = type(TypeScale.caption, 1.4f, FontWeight.Medium), color = p.soft)
            }
            val done = t.status == "completed" || t.status == "abandoned"
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    when (t.status) { "completed" -> "✓"; "in_progress" -> "◐"; "abandoned" -> "–"; "blocked" -> "!"; else -> "○" },
                    style = type(TypeScale.caption, 1.4f, family = Faces.mono),
                    color = if (t.status == "in_progress") Active else p.soft,
                    modifier = Modifier.width(12.dp),
                )
                Text(
                    t.content,
                    style = type(TypeScale.label, 1.4f).copy(textDecoration = if (done) androidx.compose.ui.text.style.TextDecoration.LineThrough else null),
                    color = if (done) p.soft else if (t.status == "in_progress") p.ink else p.mid,
                )
            }
        }
    }
}

/** One piece of a run of work: a call, or a thought between calls. */
internal sealed interface Worked {
    data class Call(val call: ToolCall) : Worked
    data class Musing(val text: String) : Worked
}

/** A run of work as one line that opens into its rows, calls and thoughts
 * in the order they came, each of which opens in turn; what is under way
 * shows below the line while it runs (App.tsx Stack). [musing] is the
 * thought still streaming; [thought] marks thinking the run doesn't hold,
 * as the chat's message keeps it apart from its calls. */
@Composable
internal fun Stack(items: List<Worked>, live: Boolean, musing: String? = null, thought: Boolean = false) {
    val p = Ink
    val detail = LocalChatDetail.current
    var open by remember(detail) { mutableStateOf(detail == "full") }
    @Composable
    fun row(w: Worked, running: Boolean) = when (w) {
        is Worked.Musing -> Thought(w.text, live = running && w.text == musing)
        is Worked.Call -> ToolRow(w.call, running && !w.call.done)
    }
    // A thought before a lone call folds with it, as the web's Stack does.
    if (items.size == 1 && !thought) {
        row(items[0], live)
        return
    }
    val busy = if (live) items.filter { if (it is Worked.Musing) it.text == musing else !(it as Worked.Call).call.done } else emptyList()
    val done = items.filter { it !in busy }
    val calls = done.filterIsInstance<Worked.Call>().map { it.call }
    val thinks = thought || calls.size < done.size
    val opened = remember { Opened() }
    Column(Modifier.padding(vertical = 6.dp).animateContentSize().revealed(opened)) {
        if (done.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).clickable { if (open) opened.close() else opened.tap(); open = !open }.padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val glyphs = ((if (thinks) listOf(Glyph.brain) else emptyList()) + calls.map { glyphOf(it.name) }).distinct().take(3)
                Box {
                    glyphs.forEachIndexed { k, g ->
                        Box(
                            Modifier.offset(x = (k * 14).dp).size(22.dp).clip(CircleShape).background(p.lift).border(1.dp, p.hair, CircleShape),
                            contentAlignment = Alignment.Center,
                        ) { Glyph(g, 12.dp, p.mid) }
                    }
                    Box(Modifier.size(width = (22 + (glyphs.size - 1) * 14).dp, height = 22.dp))
                }
                Text(
                    did(calls, thinks),
                    style = type(TypeScale.label, 1.4f, FontWeight.Medium),
                    color = p.ink.copy(alpha = 0.8f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(start = 10.dp).weight(1f),
                )
                val failed = calls.count { it.error }
                if (failed > 0) Small("$failed failed", Modifier.padding(start = 6.dp), p.accent, Type.label)
                Glyph(if (open) Glyph.chevronDown else Glyph.chevronRight, 14.dp, p.soft, Modifier.padding(start = 6.dp))
            }
            if (open) Rail { done.forEach { row(it, false) } }
        }
        if (busy.isNotEmpty()) {
            if (done.isNotEmpty()) Rail { busy.forEach { row(it, true) } } else busy.forEach { row(it, true) }
        }
    }
}

/** Rows hung from a hairline, under the line they open from. */
@Composable
private fun Rail(content: @Composable () -> Unit) {
    val hair = Ink.hair
    Column(
        Modifier.padding(start = 10.dp, top = 4.dp).drawBehind {
            drawLine(hair, Offset(0f, 0f), Offset(0f, size.height), strokeWidth = 1.dp.toPx())
        }.padding(start = 14.dp),
    ) { content() }
}

/** For a call, what opens its card full screen, drawn from what the call
 * gave back; null for a tool that has none. */
val LocalCards = compositionLocalOf<(ToolCall) -> (() -> Unit)?> { { null } }

/** One call: a spinner or a mark, its name, what it was about in mono;
 * a tap opens what went in and came out, and Open its card, if it has one. */
@Composable
fun ToolRow(t: ToolCall, running: Boolean) {
    val p = Ink
    val detail = LocalChatDetail.current
    var open by remember(detail) { mutableStateOf(detail == "full") }
    if (detail == "full") t.thought?.let { Thought(it, false) }
    val opened = remember { Opened() }
    Column(Modifier.padding(vertical = 2.dp).revealed(opened)) {
        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.md)).clickable { if (open) opened.close() else opened.tap(); open = !open }.padding(vertical = 3.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (running) {
                CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 1.5.dp, color = Active, trackColor = p.hair)
            } else {
                Glyph(glyphOf(t.name), 14.dp, if (t.error) p.accent else p.soft)
            }
            // One coloured mark a row: a failed call's glyph carries it.
            Text(labelOf(t.name), style = type(TypeScale.label, 1.4f, FontWeight.Medium), color = p.ink.copy(alpha = 0.8f))
            Text(
                about(t),
                style = type(TypeScale.caption, 1.4f, family = Faces.mono),
                color = p.soft,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            LocalCards.current(t)?.let { show ->
                Text(
                    "Open",
                    style = type(TypeScale.label, 1.4f, FontWeight.Medium),
                    color = p.accent,
                    modifier = Modifier.clip(RoundedCornerShape(Radii.md)).clickable(onClick = show).padding(horizontal = 8.dp, vertical = 2.dp),
                )
            }
            if (open) Glyph(Glyph.chevronDown, 14.dp, p.soft)
        }
        // While it runs, the last line it printed shows under the row, as a
        // terminal shows a command's; open, the row shows all of it so far.
        val printing = if (running && !t.done) t.result?.trimEnd()?.substringAfterLast('\n') else null
        if (!open && !printing.isNullOrEmpty()) {
            Text(
                printing,
                style = type(TypeScale.caption, 1.4f, family = Faces.mono),
                color = p.soft,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = 22.dp),
            )
        }
        if (open) {
            val list = if (t.name == "TodoWrite") todos(t.input) else null
            if (isMessage(t.name)) Words(said(t)) else if (list != null) Todos(list) else Code(if (detail == "full") t.arguments ?: said(t) else said(t))
            t.result?.let { Code(if (t.done) it.take(6000) else it.takeLast(6000)) }
            t.structured?.let { RichResult(it) }
            t.report?.takeIf { it != t.result }?.let { Words(it) }
        }
    }
}

/** Code, or what a call took and gave: filled in --block, as the page fills
 * what the machine wrote. */
@Composable
fun Code(text: String) {
    if (text.isBlank()) return
    val p = Ink
    Box(
        Modifier.padding(vertical = 4.dp).fillMaxWidth().clip(RoundedCornerShape(Radii.xl)).background(p.block),
    ) {
        SelectionContainer {
            Text(
                text,
                style = Type.code,
                color = p.ink,
                modifier = Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 9.dp),
            )
        }
    }
}

/** A message's words in the row's box: written, so drawn as a report is. */
@Composable
private fun Words(text: String) {
    if (text.isBlank()) return
    val p = Ink
    val shape = RoundedCornerShape(Radii.xl)
    Box(
        Modifier.padding(vertical = 4.dp).fillMaxWidth().clip(shape).background(p.lift).border(1.dp, p.hair, shape)
            .padding(horizontal = 12.dp, vertical = 9.dp),
    ) {
        Markdowned(text)
    }
}

/** The thinking: "Thinking" while it streams, "Thought" after, opening on a
 * tap to the words in --soft beside a rule (App.tsx Thought). */
@Composable
internal fun Thought(text: String, live: Boolean) {
    val p = Ink
    val detail = LocalChatDetail.current
    var open by remember(detail) { mutableStateOf(detail == "full") }
    val opened = remember { Opened() }
    Column(Modifier.padding(vertical = 6.dp).revealed(opened)) {
        Row(Modifier.clickable { if (open) opened.close() else opened.tap(); open = !open }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Glyph(Glyph.brain, 14.dp, p.soft)
            Text(if (live) "Thinking" else "Thought", style = Type.note, color = p.soft)
            Glyph(if (open) Glyph.chevronDown else Glyph.chevronRight, 12.dp, p.soft)
        }
        if (open) {
            Row(Modifier.padding(top = 4.dp)) {
                Box(Modifier.size(2.dp, 40.dp).background(p.hair))
                Text(text.trim(), style = type(TypeScale.control, 1.625f), color = p.soft, modifier = Modifier.padding(start = 12.dp))
            }
        }
    }
}

/** The assistant's words as markdown in the page's type; a draft waiting
 * for approval word for word, framed. While a reply streams, the last
 * rendering stays up as the next is parsed, so it doesn't flicker. */
@Composable
private fun Reply(text: String, plain: Boolean, files: (String) -> String) {
    val p = Ink
    if (plain) {
        Box(
            Modifier.padding(top = 8.dp).fillMaxWidth().border(1.25.dp, p.edge, RoundedCornerShape(Radii.card)).padding(horizontal = 20.dp, vertical = 14.dp),
        ) {
            SelectionContainer { Text(text, style = Type.body, color = p.ink) }
        }
        return
    }
    CompositionLocalProvider(LocalContentColor provides p.ink) {
        Markdown(
            // Parsed in the first frame: an empty first frame gives a long reply no
            // height, and a list coming back holding its place inside one is thrown
            // to the foot.
            markdownState = rememberMarkdownState(text, retainState = true, immediate = true),
            modifier = Modifier.fillMaxWidth(),
            imageTransformer = Coil3ImageTransformerImpl,
            typography = markdownTypography(
                h1 = type(22f, 1.3f, FontWeight.SemiBold),
                h2 = type(19f, 1.3f, FontWeight.SemiBold),
                h3 = type(17f, 1.35f, FontWeight.SemiBold),
                h4 = type(16f, 1.4f, FontWeight.SemiBold),
                h5 = type(16f, 1.4f, FontWeight.SemiBold),
                h6 = type(16f, 1.4f, FontWeight.SemiBold),
                text = Type.body,
                paragraph = Type.body,
                ordered = Type.body,
                bullet = Type.body,
                list = Type.body,
                code = Type.code,
                inlineCode = type(TypeScale.control, 1.55f, family = Faces.mono),
                quote = Type.body.copy(color = p.mid),
                table = Type.small,
            ),
            colors = markdownColor(
                text = p.ink,
                codeBackground = p.block,
                inlineCodeBackground = p.block,
                dividerColor = p.hair,
                tableBackground = Color.Transparent,
            ),
            padding = markdownPadding(block = 6.dp, list = 2.dp, listItemBottom = 6.dp),
        )
    }
}

/** A reply's words on a faint fill the column's width, so replies sent one
 * after another read as separate messages, as the web's SAID (App.tsx):
 * --said, rounded-2xl, px-4 py-3. The web keeps 10px (mb-2.5/mt-2.5) between
 * a fill and its neighbours but none at the message's top or foot; here the
 * neighbours (Stack, Thought) bring 6dp of their own, so a fill adds 4dp on
 * the sides that have one. */
@Composable
fun Said(top: Boolean = true, bottom: Boolean = true, words: @Composable () -> Unit) {
    Box(
        Modifier
            .padding(top = if (top) 4.dp else 0.dp, bottom = if (bottom) 4.dp else 0.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radii.xxl))
            .background(Ink.said)
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) { words() }
}

/** Words drawn as the assistant's replies are. */
@Composable
fun Markdowned(text: String) = Reply(text, false) { it }

/** While a turn runs with nothing to show yet: the book loader, its pages
 * flipping over and back, as the page draws it (BookLoader.kt). With the
 * phone's animations off it stays the shut book, a flat bar. */
@Composable
fun Dots() {
    val context = LocalContext.current
    val moving = remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f }
    val loop = rememberInfiniteTransition(label = "book")
    val t by loop.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(BookLoader.DURATION_MS, easing = LinearEasing), RepeatMode.Restart),
        label = "pages",
    )
    val color = Ink.soft
    val tall = 18.dp
    Canvas(
        Modifier.padding(vertical = 3.dp).height(tall).width(tall * (BookLoader.WIDTH / BookLoader.HEIGHT))
            .semantics { contentDescription = "Working" },
    ) {
        // Keyframes are evenly spaced; between two the stroke moves straight
        // from one to the next, as SVG's animate does.
        val at = if (moving) t * (BookLoader.FRAMES - 1) else 0f
        val a = at.toInt().coerceAtMost(BookLoader.FRAMES - 2)
        val f = at - a
        val scale = size.height / BookLoader.HEIGHT
        val stroke = Stroke(width = BookLoader.STROKE * scale, cap = StrokeCap.Round)
        for (page in BookLoader.PAGES) {
            fun v(n: Int) = page[a * 6 + n] + (page[(a + 1) * 6 + n] - page[a * 6 + n]) * f
            fun x(n: Int) = (v(n) - BookLoader.LEFT) * scale
            fun y(n: Int) = (v(n) - BookLoader.TOP) * scale
            val path = Path().apply {
                moveTo(x(0), y(1))
                quadraticTo(x(2), y(3), x(4), y(5))
            }
            drawPath(path, color, style = stroke)
        }
    }
}

/** The turn's background jobs while they run on, as the page names them. */
@Composable
fun Jobs(names: List<String>) {
    val p = Ink
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp, color = Active, trackColor = p.hair)
        Small("Working · ${if (names.size == 1) names[0] else "${names.size} jobs: ${names.joinToString(", ")}"}", maxLines = 1)
    }
}

@Composable
private fun Small(text: String, maxLines: Int) =
    Text(text, style = Type.note, color = Ink.soft, maxLines = maxLines, overflow = TextOverflow.Ellipsis)

/** A picture opened large over a dimmed screen; a tap or a pull down puts
 * it away, as a phone's gallery does. */
@Composable
fun Viewer(url: String, close: () -> Unit) {
    var drag by remember { mutableFloatStateOf(0f) }
    Dialog(onDismissRequest = close, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(
            Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = 0.82f * (1 - minOf(drag, 600f) / 800f)))
                .pointerInput(Unit) {
                    detectVerticalDragGestures(
                        onDragEnd = { if (drag > 200) close() else drag = 0f },
                        onVerticalDrag = { _, by -> drag = maxOf(0f, drag + by) },
                    )
                }
                .clickable(onClick = close)
                .padding(12.dp),
            contentAlignment = Alignment.Center,
        ) {
            AsyncImage(
                model = url,
                contentDescription = "Picture",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).offset { IntOffset(0, drag.toInt()) },
            )
            Box(
                Modifier.align(Alignment.TopEnd).size(40.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.4f)).clickable(onClick = close),
                contentAlignment = Alignment.Center,
            ) { Glyph(Glyph.close, 20.dp, Color.White, label = "Close") }
        }
    }
}
