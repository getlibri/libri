package su.libri

// The Workers tree (workTree.tsx), as a git commit graph draws its branches:
// the memories are the roots, and each worker hangs from the memory whose
// chat started it or from the worker that asked for it. Each worker is a
// card: its name and model, its title, and the status line the summarizer
// refreshes while it works. Colour says what a node is doing, the same across
// the tree (Act). A lane glows in its worker's colour while that worker
// works and is dim otherwise; it runs dashes only while a message travels
// along it between two agents, the way the message goes (task::Talk). A tap
// folds a node with workers under it and opens a leaf; a double tap opens
// either; a hold brings up the card's menu.
//
// The tree draws from /api/work and /api/memories alone. The web also draws
// a worker's harness subagents and its open call off the chat stream; the
// app reads the open call from the host's `calling` instead, and leaves the
// harness subagents out.

import android.provider.Settings
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
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import su.libri.core.NamedMemory
import su.libri.core.Talk
import su.libri.core.Thread

/** What a node is doing, as far as the app can tell, with one palette for
 * the whole tree (workTree.tsx ACTS); amber stays the person's, red the
 * failure's. Idle draws in --soft. */
enum class Act(val colour: Color?, val label: String) {
    THINKING(Color(0xFF6C8CFF), "Thinking or writing"),
    READING(Color(0xFF2BB3A3), "Reading or searching"),
    EDITING(Color(0xFFB07CFF), "Editing files"),
    RUNNING(Color(0xFF3CCF91), "Running a command or tool"),
    PERSON(Color(0xFFF2A93B), "Waiting on you"),
    AGENT(Color(0xFF8A94A8), "Waiting on an agent, or napping"),
    FAILED(Color(0xFFE5484D), "Last run failed"),
    IDLE(null, "Idle or done"),
    ;

    /** Work under way: a lane to it glows. */
    val working get() = this == THINKING || this == READING || this == EDITING || this == RUNNING
}

private fun Act.tint(p: Paper) = colour ?: p.soft

/** Tool names, as Claude Code, Codex, omp and OpenCode name them, by what
 * they do (workTree.tsx READS, EDITS). Any other call is running. */
private val READS = Regex("(^|[_.])(read|grep|glob|find|ls|list|search|fetch|view|look|zoom|cat|web_?search|web_?fetch|notebookread)", RegexOption.IGNORE_CASE)
private val EDITS = Regex("(^|[_.])(edit|multiedit|write|apply_?patch|patch|notebookedit|create_file|str_replace)", RegexOption.IGNORE_CASE)

internal fun actOfCall(tool: String): Act {
    val name = tool.replace(Regex("^mcp__\\w+?__"), "")
    return when {
        EDITS.containsMatchIn(name) -> Act.EDITING
        READS.containsMatchIn(name) -> Act.READING
        else -> Act.RUNNING
    }
}

/** What worker `t` is doing (workTree.tsx actOf). */
internal fun actOf(t: Thread): Act = when (t.state) {
    "waiting" -> if (t.failed) Act.FAILED else Act.PERSON
    "working" -> when {
        t.waitingOn.isNotEmpty() -> Act.AGENT
        else -> t.calling?.let(::actOfCall) ?: Act.THINKING
    }
    "napping" -> Act.AGENT
    else -> if (t.failed) Act.FAILED else Act.IDLE
}

/** A worker's harness, its model by the name the model picker shows it
 * (ModelNames.kt), then its thinking level (workTree.tsx shortModel). */
internal fun shortModel(t: Thread, offers: List<su.libri.core.Offer>): String? {
    val pick = t.pick ?: return null
    return listOfNotNull(pick.harness, modelName(pick.harness, pick.model, offers).ifEmpty { null }, t.effort).joinToString(" · ")
}

private class Node(
    val key: String,
    val name: String,
    val memory: Boolean,
    val act: Act,
    val kids: List<Node>,
    val thread: Thread? = null,
    val model: String? = null,
    val title: String? = null,
    val status: String? = null,
)

private class Row(val n: Node, val depth: Int, val top: Dp, val height: Dp, val parent: Row?) {
    /** A memory's own workers sit on its line, as commits sit on their
     * branch; only a worker's sub-workers step in, one column a level. A
     * row's words start a column past its node, where the lane to its own
     * sub-workers runs down, so that lane never crosses them. */
    val column: Int get() = maxOf(0, depth - 1)
}

/** How long a lane runs its dashes for one message, in milliseconds. */
private const val TALK_SHOWN = 4000L
private const val TALK_FORGOT = 30_000L

private val CARD = 70.dp
private val ROOT = 36.dp
private val X0 = 14.dp
private val DX = 20.dp

/** The tree: Main and each named memory, the workers under them. Archived
 * workers stay out, on the tile's second tab. */
@Composable
fun WorkTree(
    model: ChatModel,
    main: Act,
    open: (Thread) -> Unit,
    openMain: () -> Unit,
    hold: (Thread, LayoutCoordinates) -> Unit,
) {
    val p = Ink
    val threads = model.threads.filter { it.state != "archived" }
    val ids = threads.map { it.id }.toSet()
    fun worker(t: Thread): Node {
        val act = actOf(t)
        val tag = when {
            t.state == "waiting" && t.need != null -> NEED[t.need]
            act == Act.FAILED -> "Failed"
            t.waitingOn.isNotEmpty() -> "Waiting on ${t.waitingOn.joinToString(", ")}"
            else -> t.state.replaceFirstChar { it.uppercase() }
        }
        return Node(
            key = t.id, name = t.name, memory = false, act = act, thread = t,
            kids = threads.filter { it.head == t.id }.map(::worker),
            model = shortModel(t, model.models?.offers.orEmpty()),
            title = t.title ?: firstLine(t.brief),
            // While it works, the summarizer's line; after, its report or its ask.
            status = listOfNotNull(tag, (t.summary ?: t.gist).takeIf { it.isNotBlank() }).joinToString(" · "),
        )
    }
    // A worker hangs from the memory it belongs to; one whose memory was
    // deleted, from Main, so it stays in reach.
    val named = model.memories.map { it.id }.toSet()
    fun top(memory: String?) = threads.filter { (it.head == null || it.head !in ids) && (it.memory?.takeIf { m -> m in named }) == memory }.map(::worker)
    fun chatAct(m: NamedMemory) = if (m.asking) Act.PERSON else if (m.working) Act.THINKING else Act.IDLE
    val roots = listOf(Node("memory:main", "Main", true, main, top(null))) +
        model.memories.map { Node("memory:${it.id}", it.name, true, chatAct(it), top(it.id)) }

    val folded = { key: String -> !model.sectionOpen("tree/$key", true) }
    val rows = mutableListOf<Row>()
    var y = 0.dp
    fun walk(list: List<Node>, d: Int, parent: Row?) {
        for (n in list) {
            val h = if (n.memory) ROOT else CARD
            val r = Row(n, d, y, h, parent)
            y += h
            rows += r
            if (!folded(n.key)) walk(n.kids, d + 1, r)
        }
    }
    walk(roots, 0, null)
    val shown = rows.associateBy { it.n.key }
    val parentOf = HashMap<String, String>()
    fun link(list: List<Node>) { for (n in list) for (k in n.kids) { parentOf[k.key] = n.key; link(listOf(k)) } }
    link(roots)

    // Each message with when the app first saw it, kept while it shows.
    val seen = remember { mutableStateMapOf<String, Long>() }
    var now by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(model.talk) {
        val at = System.currentTimeMillis()
        for (t in model.talk) seen.putIfAbsent("${t.at}|${t.from}|${t.to}", at)
        now = at
    }
    val live = seen.filterValues { now - it < TALK_SHOWN }
    LaunchedEffect(live.keys) {
        if (live.isEmpty()) return@LaunchedEffect
        delay(live.values.min() + TALK_SHOWN - System.currentTimeMillis() + 50)
        now = System.currentTimeMillis()
        // Kept past the host's own window on talk (task::TALK_NEWS_MS), so a
        // message read again isn't taken for a new one.
        for (k in seen.filterValues { now - it > TALK_FORGOT }.keys) seen.remove(k)
    }
    // The lanes a message runs along: up from its sender to where the two
    // branches meet, then down to whom it is for, each lane keyed by the row
    // it ends at. A lane folded away shows nothing.
    val flows = HashMap<String, Boolean>() // true: down, the lane's way
    for (k in live.keys) {
        val (_, from, to) = k.split('|', limit = 3)
        fun chain(key: String) = generateSequence(key) { parentOf[it] }.toList()
        val up = chain(from)
        val down = chain(to)
        val meet = up.firstOrNull { it in down } ?: continue
        for (n in up.subList(0, up.indexOf(meet))) if (shown[n]?.parent != null) flows[n] = false
        for (n in down.subList(0, down.indexOf(meet))) if (shown[n]?.parent != null) flows[n] = true
    }

    val context = LocalContext.current
    val moving = remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f }
    val beat = rememberInfiniteTransition(label = "tree")
    val glow by beat.animateFloat(0.35f, 0.8f, infiniteRepeatable(tween(800), RepeatMode.Reverse), label = "glow")
    val dash by beat.animateFloat(0f, 1f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "dash")
    // A node sits level with its name, the first line of its card.
    fun at(r: Row) = Pair(X0 + DX * r.column, r.top + if (r.n.memory) r.height / 2 else 17.dp)

    Column(Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth().height(y).semantics { contentDescription = "Workers by memory" }) {
            // The rows first, the graph over them, so a row's press shows under its dot.
            Column(Modifier.fillMaxWidth()) {
                for (r in rows) NodeRow(r, folded(r.n.key), p, toggle = { model.flipSection("tree/${r.n.key}", true) }, open = {
                    r.n.thread?.let(open) ?: when {
                        r.n.key == "memory:main" -> openMain()
                        // A named memory's own chat, as a tab of the chat (MemoryTabs.kt).
                        r.n.key.startsWith("memory:") -> model.memoryTabAsked = r.n.key.removePrefix("memory:")
                        else -> Unit
                    }
                }, hold = hold)
            }
            Canvas(Modifier.fillMaxWidth().height(y)) {
                val period = 7.dp.toPx() * 2
                // Out of the parent along a smooth fork to this one's column,
                // then down it; on the parent's own column, straight down.
                fun lane(r: Row, pr: Row): Path {
                    val (cx, cy) = at(r).let { Offset(it.first.toPx(), it.second.toPx()) }
                    val (px, py) = at(pr).let { Offset(it.first.toPx(), it.second.toPx()) }
                    return Path().apply {
                        moveTo(px, py)
                        cubicTo(cx, py, cx, py, cx, py + 14.dp.toPx())
                        lineTo(cx, cy)
                    }
                }
                // Idle lanes first, working ones over them, dashes last.
                for (r in rows.sortedBy { it.n.act.working }) {
                    val path = lane(r, r.parent ?: continue)
                    val c = r.n.act.tint(p)
                    if (r.n.act.working) {
                        drawPath(path, c.copy(alpha = if (moving) glow * 0.45f else 0.3f), style = Stroke(7.dp.toPx(), cap = StrokeCap.Round))
                        drawPath(path, c, style = Stroke(2.dp.toPx(), cap = StrokeCap.Round))
                    } else drawPath(path, p.frame, style = Stroke(2.dp.toPx(), cap = StrokeCap.Round))
                }
                for (r in rows) {
                    val down = flows[r.n.key] ?: continue
                    val phase = if (!moving) 0f else if (down) period * (1 - dash) else period * dash
                    drawPath(lane(r, r.parent ?: continue), p.ink, style = Stroke(2.5.dp.toPx(), cap = StrokeCap.Round, pathEffect = PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(), 5.dp.toPx() + 2.5.dp.toPx()), phase)))
                }
                for (r in rows) {
                    val (x, y0) = at(r)
                    val o = Offset(x.toPx(), y0.toPx())
                    val act = r.n.act
                    val c = act.tint(p)
                    val size = (if (r.n.memory) 7.dp else 5.5.dp).toPx()
                    // Working shows by a breathing glow; a wait or a failure holds still.
                    if (act.working) drawCircle(c.copy(alpha = if (moving) glow * 0.5f else 0.35f), size + 4.dp.toPx(), o)
                    if (act == Act.IDLE) {
                        drawCircle(p.paper, size, o)
                        drawCircle(c.copy(alpha = 0.7f), size, o, style = Stroke(2.dp.toPx()))
                    } else drawCircle(c, size, o)
                }
            }
        }
    }
}

private fun count(n: Node): Int = n.kids.sumOf { 1 + count(it) }

@Composable
private fun NodeRow(r: Row, shut: Boolean, p: Paper, toggle: () -> Unit, open: () -> Unit, hold: (Thread, LayoutCoordinates) -> Unit) {
    val n = r.n
    val parent = n.kids.isNotEmpty()
    var where by remember { mutableStateOf<LayoutCoordinates?>(null) }
    val t = n.thread
    val line = type(TypeScale.caption, 1.3f)
    Row(
        Modifier.fillMaxWidth().height(r.height)
            .onGloballyPositioned { where = it }
            .combinedClickable(
                // A leaf has nothing to fold, so a tap opens it.
                onClick = if (parent) toggle else open,
                onDoubleClick = if (parent) open else null,
                onLongClickLabel = "Actions",
                onLongClick = if (t == null || (t.state == "working" && t.nap == null)) null else ({ where?.let { hold(t, it) } }),
            )
            .semantics { stateDescription = n.act.label }
            .padding(start = X0 + DX * (r.column + 1) + 10.dp, end = 12.dp, top = if (n.memory) 0.dp else 7.dp),
        verticalAlignment = if (n.memory) Alignment.CenterVertically else Alignment.Top,
    ) {
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(n.name, style = line.copy(fontWeight = FontWeight.SemiBold), color = p.ink, maxLines = 1)
                // What the row is, and what folds under it, as a tree view's
                // twistie and badge say.
                val under = listOfNotNull(
                    "chat".takeIf { n.memory },
                    count(n).takeIf { parent }?.let { c -> "$c ${if (n.memory) "worker" else "sub-worker"}${if (c == 1) "" else "s"}" },
                ).joinToString(" · ")
                if (under.isNotEmpty()) Text(under, style = line, color = p.soft, maxLines = 1, modifier = Modifier.padding(start = 6.dp))
                if (parent) Glyph(if (shut) Glyph.chevronRight else Glyph.chevronDown, 12.dp, p.soft, Modifier.padding(start = 4.dp))
                n.model?.let { Text(it, style = line.copy(fontFamily = Faces.mono, fontSize = 11.sp), color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 8.dp).weight(1f, fill = false)) }
                // The harness ran on another model or level than it was started on.
                n.thread?.mismatch?.let {
                    Text("mismatch", style = line.copy(fontFamily = Faces.mono, fontSize = 11.sp), color = Act.FAILED.colour ?: p.ink, maxLines = 1, modifier = Modifier.padding(start = 6.dp).semantics { contentDescription = "Model mismatch: $it" })
                }
            }
            if (!n.memory) {
                n.title?.let { Text(it, style = line, color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                n.status?.takeIf { it.isNotEmpty() }?.let {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.padding(end = 6.dp).size(6.dp).background(n.act.tint(p), CircleShape))
                        Text(it, style = line, color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
        t?.let { Text(ago(it.at), style = line, color = p.soft, maxLines = 1, modifier = Modifier.padding(start = 10.dp)) }
    }
}

/** The Workers tile's tabs, by id. */
val WORKERS_TABS = listOf("tree", "watches", "archived")

/** The Workers tile's tabs: the tree, the watches, and the archived, which
 * shows its count only once opened so old workers never read as something
 * to do; on the tree's, the colours' key at the row's end, named as one,
 * which opens to what each colour means (workTree.tsx `WorkersTile`). */
@Composable
fun WorkTabs(tab: String, workers: Int, watches: Int, show: (String) -> Unit) {
    val p = Ink
    var keyOpen by remember { mutableStateOf(false) }
    val line = type(TypeScale.caption, 1.3f)
    Column(Modifier.fillMaxWidth().padding(bottom = 4.dp), horizontalAlignment = Alignment.End) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            for ((id, label, count) in listOf(Triple("tree", "Workers", workers), Triple("watches", "Watches", watches), Triple("archived", "Archived", 0))) {
                val picked = id == tab
                Row(
                    Modifier.padding(end = 4.dp).height(32.dp).clip(RoundedCornerShape(50))
                        .then(if (picked) Modifier.background(p.hover).border(1.dp, p.hair, RoundedCornerShape(50)) else Modifier)
                        .selectable(selected = picked, role = Role.Tab) { show(id) }
                        .padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(label, style = line.copy(fontWeight = if (picked) FontWeight.Medium else FontWeight.Normal), color = if (picked) p.ink else p.soft, maxLines = 1)
                    if (count > 0) Text("$count", style = line, color = p.soft, modifier = Modifier.padding(start = 6.dp))
                }
            }
            Spacer(Modifier.weight(1f))
            if (tab == "tree") Row(
                Modifier.height(32.dp).clip(RoundedCornerShape(50)).clickable(onClickLabel = "What the colours mean") { keyOpen = !keyOpen }.padding(horizontal = 10.dp)
                    .semantics { contentDescription = "What the colours mean" },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    for (half in Act.entries.chunked(4)) Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                        for (a in half) Box(Modifier.size(6.dp).background(a.tint(p), CircleShape))
                    }
                }
                Text("Key", style = line, color = p.soft, modifier = Modifier.padding(start = 6.dp))
            }
        }
        if (tab == "tree" && keyOpen) Column(Modifier.padding(top = 4.dp).card(p, radius = Radii.lg).padding(horizontal = 12.dp, vertical = 8.dp)) {
            for (a in Act.entries) Row(Modifier.padding(vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(10.dp).background(a.tint(p), CircleShape))
                Text(a.label, style = line, color = p.ink, modifier = Modifier.padding(start = 8.dp))
            }
            Row(Modifier.padding(vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Canvas(Modifier.size(10.dp)) {
                    drawLine(p.ink, Offset(size.width / 2, 0f), Offset(size.width / 2, size.height), 2.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(), 3.dp.toPx())))
                }
                Text("A message between two agents", style = type(TypeScale.caption, 1.3f, FontWeight.Normal), color = p.ink, modifier = Modifier.padding(start = 8.dp))
            }
        }
    }
}
