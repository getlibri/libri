package su.libri

import android.provider.Settings
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.height
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.withContext
import su.libri.core.Message
import su.libri.core.Outbox
import su.libri.core.Role
import su.libri.core.Step
import su.libri.core.StepKind
import su.libri.core.Thread
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import su.libri.core.Offer
import su.libri.core.ToolCall
import su.libri.core.WorkChoice

/** How long ago, as a card says it (work.tsx `ago`). */
fun ago(secs: ULong): String {
    val at = secs.toLong()
    val s = maxOf(0L, System.currentTimeMillis() / 1000 - at)
    return when {
        s < 60 -> "now"
        s < 3600 -> "${s / 60}m"
        s < 86400 -> "${s / 3600}h"
        s < 86400 * 7 -> "${s / 86400}d"
        else -> SimpleDateFormat("MMM d", Locale.getDefault()).format(Date(at * 1000))
    }
}

/** The Workers tile: on its first tab the tree of who works and waits, on
 * its second the watches and the archived. Asked again while in view. */
@Composable
fun WorkPlace(model: ChatModel, bring: (Thread) -> Unit, openMain: () -> Unit, modifier: Modifier = Modifier) {
    // A thread opens in this page's place, the bar still under it.
    var opened by model::opened
    var heldWatch by remember { mutableStateOf<HeldWatch?>(null) }
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(owner) {
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.refreshWatches()
                delay(30_000)
            }
        }
    }
    val chat by model.chat.collectAsStateWithLifecycle()
    val steps = chat?.workerSteps.orEmpty()
    val all = model.threads
    val napping = all.filter { it.state == "napping" }.sortedBy { it.nap ?: 0u }
    val put = all.filter { it.state == "archived" }
    val p = Ink
    // A nap ends on the host's tick; the list asks again as the first one due
    // passes (work.tsx useWork).
    val due = napping.firstOrNull()?.nap
    LaunchedEffect(due) {
        if (due == null) return@LaunchedEffect
        delay((due.toLong() * 1000 - System.currentTimeMillis()).coerceAtLeast(0) + 1500)
        model.refreshWork()
    }
    Box(modifier.fillMaxSize()) {
        // No head, as on Chat: the bar names the place and each section's
        // head carries its count; Mark all read sits on Waiting's head.
        Column(Modifier.fillMaxSize()) {
            // The list runs up under the status bar (Places' "workers").
            val top = statusTop
            model.started?.let { Text(it, style = Type.note, color = p.mid, modifier = Modifier.padding(start = 18.dp, end = 14.dp, top = top + 12.dp)) }
            val under = if (model.started == null) top else 0.dp
            LazyColumn(Modifier.fillMaxSize().edgeFade(p.paper, top = if (under > 0.dp) under + TOP_FADE else 0.dp, bottom = LocalBarRoom.current + 24.dp), contentPadding = PaddingValues(start = 14.dp, end = 14.dp, top = under + 24.dp, bottom = FAB_CLEARANCE + LocalBarRoom.current), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val tab = model.workersTab
                item(key = "tabs") { WorkTabs(tab, all.size - put.size, model.watches.size, model::showWorkersTab) }
                val empty: (String) -> Unit = { words ->
                    item(key = "empty") {
                        Text(words, style = type(TypeScale.control, 1.5f), color = p.mid, modifier = Modifier.fillMaxWidth().dashed(p.frame).padding(horizontal = 20.dp, vertical = 24.dp))
                    }
                }
                val card: @Composable (Thread) -> Unit = { t -> Card(t, steps[t.id] ?: 0u, model, open = { opened = t.id }) }
                // Who works and waits, as the tree draws it (WorkTree.kt); the
                // watches and the archived each on a tab of its own, as on the web.
                if (tab == "tree") {
                    if (all.any { it.state != "archived" } || model.memories.isNotEmpty()) item(key = "tree") {
                        val main = when {
                            chat?.asks?.isNotEmpty() == true -> Act.PERSON
                            chat?.typing == true -> Act.THINKING
                            else -> Act.IDLE
                        }
                        WorkTree(
                            model, main,
                            open = { t -> opened = t.id; if (t.state == "waiting") model.workAct(t.id, "read") },
                            openMain = openMain,
                            hold = { t, at -> model.heldCard = HeldCard(t.id, at.boundsInWindow()) },
                        )
                    }
                    else empty("Nothing here yet. Hold a message and pick Create task, or type /work.")
                } else if (tab == "watches") {
                    if (model.watches.isEmpty()) empty("No watches.")
                    else watchList(model) { heldWatch = it }
                } else if (put.isEmpty()) empty("No archived workers.")
                else {
                    // The count the tab leaves off, once the tab is open.
                    item(key = "archived-count") { Head("${put.size} archived", color = p.soft, modifier = Modifier.padding(start = 4.dp, top = 6.dp)) }
                    items(put, key = { "archived/${it.id}" }) { card(it) }
                }
            }
        }
        // + opens the Tasks sheet (TaskSheet.kt): a worker on the person's own
        // words, not through the main chat.
        Fab("Start a worker", open = model.newTask) { model.newTask = true }
    }
    heldWatch?.let { h -> WatchMenu(model, h) { heldWatch = null } }
    // Read from the live list, so a card that moves on takes its menu with it.
    model.heldCard?.let { h ->
        val context = LocalContext.current
        // Nap turns the menu to the nap's times, as T3's snooze submenu does.
        var napStep by remember(h) { mutableStateOf(false) }
        val close = { model.heldCard = null }
        val t = all.firstOrNull { it.id == h.id }
        val items = t?.let {
            if (napStep) napItems(it, model, context, close) else offers(it, model, bring, close) { napStep = true }
        }.orEmpty()
        if (t == null || items.isEmpty()) LaunchedEffect(h) { close() }
        else HoldMenu(h.bounds, framed = false, items, close) { CardFace(t, steps[t.id] ?: 0u) }
    }
}

private fun Modifier.dashed(c: Color) = this.drawBehind {
    drawRoundRect(c, cornerRadius = CornerRadius(16.dp.toPx()), style = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx()))))
}

/** A thread's state in a word (work.tsx STATE). */
private val STATE = mapOf("working" to "Working", "waiting" to "Waiting", "napping" to "Napping", "sleeping" to "Sleeping", "archived" to "Archived")

/** A Waiting card's tag, what it waits for (work.tsx NEED). */
internal val NEED = mapOf("review" to "For review", "approval" to "For approval", "answer" to "For an answer")

/** Whether a Waiting card asks the person to do more than read it (work.tsx asks). */
internal fun asks(t: Thread) = t.state == "waiting" && (t.need == "approval" || t.need == "answer")

/** Who a paused thread waits on: the one part of its state no section says (work.tsx waitingOn). */
private fun waitingOn(t: Thread) = if (t.waitingOn.isEmpty()) null else "Waiting on ${t.waitingOn.joinToString(", ")}"

/** A thread's state in words, naming who a paused one waits on (work.tsx stateOf). */
private fun stateOf(t: Thread) = waitingOn(t) ?: STATE[t.state] ?: t.state

/** Markdown as a line of plain text: its first line that says anything, the
 * marks taken off. A code fence's own line says nothing (chat.ts firstLine). */
fun firstLine(text: String) =
    text.lineSequence().firstOrNull { it.isNotBlank() && !FENCE.matches(it) }.orEmpty()
        .replace(Regex("^\\s*((#+|[-*+]|\\d+[.)])\\s+|>\\s*)*"), "")
        .replace(Regex("!?\\[([^\\]]*)\\]\\([^)]*\\)"), "$1")
        .replace(Regex("(?<![\\w*~])(\\*\\*?|__?|~~)(?=\\S)(.+?)(?<=\\S)\\1(?![\\w*~])"), "$2")
        .replace("`", "")

private val FENCE = Regex("\\s*(```|~~~)\\S*\\s*")

/** A card's dot, in a box of its own so the names line up: red while it
 * waits, --frame at rest, and for a worker at work green, a ring of it
 * spreading and fading behind (Tailwind's animate-ping), left out when the
 * phone's animations are off, or while it is paused on another. */
@Composable
private fun CardDot(state: String, paused: Boolean) {
    val p = Ink
    val context = LocalContext.current
    val moving = remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f }
    val said = STATE[state] ?: state
    Box(Modifier.size(10.dp).semantics { contentDescription = said }, contentAlignment = Alignment.Center) {
        if (state == "working" && moving && !paused) {
            val t by rememberInfiniteTransition(label = "ping").animateFloat(
                initialValue = 0f,
                targetValue = 1f,
                animationSpec = infiniteRepeatable(
                    keyframes {
                        durationMillis = 1000
                        0f at 0 using CubicBezierEasing(0f, 0f, 0.2f, 1f)
                        1f at 750
                    },
                ),
                label = "ping-ring",
            )
            Box(
                Modifier.size(10.dp).graphicsLayer {
                    scaleX = 1 + t
                    scaleY = 1 + t
                    alpha = 1 - t
                }.background(Active.copy(alpha = 0.4f), CircleShape),
            )
        }
        val dot = when (state) {
            "waiting" -> p.accent
            "working" -> Active
            else -> p.frame
        }
        // A ring while it naps (work.tsx Dot).
        if (state == "napping") Box(Modifier.size(8.dp).border(1.5.dp, p.soft, CircleShape))
        else Box(Modifier.size(8.dp).background(dot, CircleShape))
    }
}

/** A Work card held: which thread, and where it stood on screen. */
data class HeldCard(val id: String, val bounds: Rect)

/** What a card offers from a hold; a Working one only the cancel of a nap it
 * keeps for later (work.tsx `offers`). */
private fun offers(t: Thread, model: ChatModel, bring: (Thread) -> Unit, close: () -> Unit, toNap: () -> Unit) = buildList<Triple<String, ImageVector, () -> Unit>> {
    val wake = Triple("Wake now", Glyph.sun) { model.workAct(t.id, "wake"); close() }
    when (t.state) {
        // That nap hasn't begun, so the item cancels it.
        "working" -> if (t.nap != null) add(Triple("Cancel nap", Glyph.close) { model.workAct(t.id, "wake"); close() })
        "archived" -> add(Triple("Restore", Glyph.restore) { model.workAct(t.id, "restore"); close() })
        else -> {
            if (t.state == "waiting") add(Triple("Mark read", Glyph.check) { model.workAct(t.id, "clear"); close() })
            add(if (t.state == "napping") wake else Triple("Nap", Glyph.moon, toNap))
            add(Triple("Bring to chat", Glyph.chat) { model.workAct(t.id, "read"); bring(t); close() })
            add(Triple("Archive", Glyph.archive) { model.workAct(t.id, "archive"); close() })
        }
    }
}

/** The nap's times, then a time of the person's own from the phone's own
 * date and time pickers (work.tsx CardMenu). */
private fun napItems(t: Thread, model: ChatModel, context: android.content.Context, close: () -> Unit) = buildList<Triple<String, ImageVector, () -> Unit>> {
    napPresets().forEach { n -> add(Triple(n.label, Glyph.moon) { model.workNap(t.id, n.until); close() }) }
    add(Triple("Pick a time", Glyph.clock) {
        close()
        val c = java.util.Calendar.getInstance().apply { add(java.util.Calendar.HOUR_OF_DAY, 1) }
        android.app.DatePickerDialog(context, { _, y, m, d ->
            android.app.TimePickerDialog(context, { _, h, min ->
                c.set(y, m, d, h, min, 0)
                if (c.timeInMillis > System.currentTimeMillis()) model.workNap(t.id, c.timeInMillis / 1000)
                else android.widget.Toast.makeText(context, "That time has passed. Choose another.", android.widget.Toast.LENGTH_SHORT).show()
            }, c.get(java.util.Calendar.HOUR_OF_DAY), c.get(java.util.Calendar.MINUTE), android.text.format.DateFormat.is24HourFormat(context)).show()
        }, c.get(java.util.Calendar.YEAR), c.get(java.util.Calendar.MONTH), c.get(java.util.Calendar.DAY_OF_MONTH)).apply {
            datePicker.minDate = System.currentTimeMillis() - 1000
        }.show()
    })
}

/** A thread's card: opens on a tap, and a hold brings up what it offers. */
@Composable
private fun Card(t: Thread, steps: UInt, model: ChatModel, open: () -> Unit) {
    var where by remember { mutableStateOf<LayoutCoordinates?>(null) }
    CardFace(
        t,
        steps,
        Modifier.onGloballyPositioned { where = it }.combinedClickable(
            onClick = open,
            onLongClickLabel = "Actions",
            onLongClick = if (t.state == "working" && t.nap == null) null else ({ where?.let { model.heldCard = HeldCard(t.id, it.boundsInWindow()) } }),
        ),
    )
}

/** A card as it reads (work.tsx WorkList), three rows in every state and none
 * ever wraps: a dot (red pencil while it has news, --frame when read), its
 * name, its steps or who it waits on, and its age; the title in 13px medium;
 * what the host says of it in 12px --mid. */
@Composable
private fun CardFace(t: Thread, steps: UInt, modifier: Modifier = Modifier) {
    val p = Ink
    val working = t.state == "working"
    val line = type(TypeScale.caption, 15.525f / TypeScale.caption)
    Column(Modifier.fillMaxWidth().card(p, radius = Radii.xl, framed = false).border(1.dp, p.hair, RoundedCornerShape(Radii.xl)).then(modifier).padding(horizontal = 12.dp, vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CardDot(t.state, t.waitingOn.isNotEmpty())
            Text(t.name, style = line.copy(fontWeight = FontWeight.SemiBold), color = p.ink, maxLines = 1, modifier = Modifier.padding(start = 6.dp))
            val tag = when {
                t.state == "waiting" -> t.need?.let { NEED[it] }
                t.state == "archived" && t.unlanded.isNotEmpty() -> "Unlanded work"
                else -> null
            }
            if (tag != null) Text(
                tag, style = line.copy(fontWeight = FontWeight.Medium), color = p.accent, maxLines = 1,
                modifier = Modifier.padding(start = 6.dp).background(p.accent.copy(alpha = 0.1f), RoundedCornerShape(4.dp)).padding(horizontal = 6.dp),
            )
            Text(
                listOfNotNull(
                    waitingOn(t),
                    t.nap?.takeIf { t.state == "napping" }?.let { "wakes ${wakeWords(it.toLong())}" },
                    t.nap?.takeIf { t.state == "working" }?.let { "naps till ${wakeWords(it.toLong())}" },
                    if (working && steps > 0u) "$steps step${if (steps == 1u) "" else "s"}" else null,
                ).joinToString(" · "),
                style = line, color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 6.dp).weight(1f),
            )
            Text(ago(t.at), style = line, color = p.soft, maxLines = 1, modifier = Modifier.padding(start = 6.dp))
        }
        Text(t.title ?: firstLine(t.brief), style = type(TypeScale.label, 1.35f, FontWeight.Medium), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
        Text(t.gist, style = type(TypeScale.caption, 1.35f), color = p.mid, minLines = 1, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
    }
}

/** How the host marks the person's words in a thread's log; main's tell_agent
 * words come bare. */
private const val TYPED = "(From the person, typed on your page) "

/** The device mark the host puts before the person's words when they change
 * device (libri/src/origin.rs), taken off for the bubble. */
private val DEVICE_MARK = Regex("^\\(from (?:android|desktop|web-phone|web-desktop|other)\\) ")

/** The person's own words of a told line: without the page's mark or the device's. */
private fun personWords(text: String) = text.removePrefix(TYPED).replaceFirst(DEVICE_MARK, "")

/** Whether this told line carries the person's `words`. Words and files arrive
 * as "(sent a photo: x) words", and a text file's body follows the words. */
private fun Step.carries(words: String, plain: Boolean): Boolean {
    if (!plain && !text.startsWith(TYPED)) return false
    val said = personWords(text)
    return said.startsWith(words) || said.startsWith("(sent ") && ") $words" in said
}

/** Words told a thread, until its log has them: a sleeper's at once, a running
 * one's when it reads them between its calls. `id` is its place in the list,
 * as a stand-in can leave from anywhere in it. */
class Telling(val text: String, val id: Int)

/** What an open thread's page keeps while the page itself is gone: its unsent
 * words, its scroll, and the log that scroll stands in, since a list that comes
 * back empty would lose its place. `plain`: only the person writes on its log,
 * so its lines carry no mark of who typed them. */
class ThreadView(val draft: Draft, private val plain: Boolean = false) {
    val list = LazyListState()
    // As the chat's: what a tool row or the dots give back is held (Follow).
    val follow = Follow(holding = true)
    var steps by mutableStateOf<List<Step>?>(null)
    val telling = mutableStateListOf<Telling>()
    val reading = Mutex()

    private var tells = 0

    fun tell(text: String) = Telling(text, tells++).also { telling += it }

    /** Takes a newer log. Each told line it gains ends the stand-in with its
     * words, since a later tell can land first. Main's lines and the person's
     * from elsewhere end none, nor does a line of files alone: it has no
     * stand-in. */
    fun heard(now: List<Step>) {
        val had = steps?.count { it.kind == StepKind.TOLD } ?: 0
        steps = now
        now.filter { it.kind == StepKind.TOLD }.drop(had).forEach { line ->
            telling.firstOrNull { line.carries(it.text, plain) }?.let { telling.remove(it) }
        }
    }
}

/** One thread's page (App.tsx AgentPage): a 51px head with its name, whether
 * it works and the way back; its task as the person's words, then its words
 * and calls in order and its report; three dots while it works; and the same
 * composer as the chat's, with this thread's own model, thinking and access,
 * to tell it more. Opens at the latest. */
@Composable
fun ThreadPage(model: ChatModel, id: String, shown: Boolean, close: () -> Unit) {
    val p = Ink
    // Held by the model: a terminal or a call takes this page down.
    val view = model.threadView(id)
    val draft = view.draft
    var choice by remember(id) { mutableStateOf<WorkChoice?>(null) }
    var stopping by remember { mutableStateOf(false) }
    var merging by remember(id) { mutableStateOf(false) }
    val thread = model.threads.find { it.id == id }
    val name = thread?.name ?: id
    val working = thread?.state == "working"
    // In view as it comes to wait, it has been seen; open behind another
    // place it has not. Told again, it can be stopped again.
    LaunchedEffect(thread?.state, shown) {
        // A question or an approval stays until given: reading it is not that.
        if (shown && thread?.state == "waiting" && !asks(thread)) model.workAct(id, "read")
        if (!working) stopping = false
    }
    val list = view.list
    val follow = view.follow
    // Asked again on coming back into view: it may have changed elsewhere meanwhile.
    LaunchedEffect(id, shown) {
        if (shown) choice = model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.workChoice(id) } }.getOrNull() } ?: choice
    }
    val scope = rememberCoroutineScope()
    val choose = { o: Offer?, effort: String?, access: String?, sandbox: String? ->
        scope.launch {
            val h = model.host
            runCatching { withContext(Dispatchers.IO) { h?.workChoose(id, o?.harness, o?.model, effort, access, sandbox) } }
                .onSuccess { choice = it ?: choice }
                .onFailure { model.notice = it.message }
        }
        Unit
    }
    val owner = LocalLifecycleOwner.current
    LaunchedEffect(id, shown, owner) {
        if (!shown) return@LaunchedEffect
        owner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                model.readLog(id)
                model.refreshWork()
                delay(3000)
            }
        }
    }
    if (merging) {
        val away = if (thread?.state == "archived") "" else " $name is then archived."
        ConfirmDialog(
            title = "Merge $name into main?",
            body = "All $name did since it started, its tool calls and their output too, joins the main chat's memory as though done there.$away This can't be undone.",
            confirm = "Merge",
            danger = true,
            onDismiss = { merging = false },
            onConfirm = { merging = false; model.workMerge(id, name) },
        )
    }
    BoxWithConstraints(Modifier.fillMaxSize().background(p.paper)) {
    val room = maxHeight
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().height(51.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                buildAnnotatedString {
                    append(name)
                    withStyle(SpanStyle(fontWeight = FontWeight.Normal, color = p.mid)) { append(" · ${thread?.let { stateOf(it).replaceFirstChar(Char::lowercase) } ?: "done"}") }
                },
                style = type(TypeScale.body, 1.3f, FontWeight.SemiBold), color = p.ink, maxLines = 1, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(start = 8.dp),
            )
            // As the web's AgentPage: a thread whose job is over merges into
            // main, archived ones too; not one at work, paused on a reply, or
            // asking the person something, which the host refuses; nor one
            // merged that has done nothing since, the host says.
            if (thread != null && !working && !asks(thread) && thread.waitingOn.isEmpty() && !thread.merged) {
                Round(onClick = { merging = true }, size = 36.dp) { Glyph(Glyph.merge, 18.dp, p.mid, label = "Merge into main") }
            }
            // Down, as the page folds a thread back into its list.
            Round(onClick = close, size = 36.dp) { Glyph(Glyph.down, 18.dp, p.mid, label = "Back to Tasks") }
        }
        model.landed?.takeIf { it.first == id }?.let { (_, said) ->
            Text(said, style = Type.label, color = p.mid, modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp))
        }
        // The composer floats over the talk's foot, as the chat's dock
        // (LibriApp.kt ChatDock).
        val dock = remember { DockRoom() }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            val all = view.steps
            val messages = logMessages(all.orEmpty(), view.telling, working, model.threadMessages(id).ifEmpty { model.threadMessages(name) }, model.chatDetail)
            ChatTalk(
                model, messages, list, follow, typing = working || all == null, draft = draft,
                fadeTop = 16.dp,
                // 24dp above the dock: 12 here and the 12 spacing the held blank stands apart by.
                padding = PaddingValues(start = 14.dp, end = 14.dp, top = 24.dp, bottom = dock.under + 12.dp),
                spacing = 12.dp,
                under = dock.under,
                floor = dock.floor,
                waiting = false,
                again = { text, _ -> model.tell(id, text, emptyList()) },
                empty = if (all != null && !working) ({ Text("Its work wasn't kept.", style = Type.body, color = p.mid) }) else null,
            )
        // Temp's and the chat's input; one trailing button, as theirs.
        ChatDock(dock) {
        Box(Modifier.padding(start = 14.dp, end = 14.dp, top = 6.dp, bottom = 8.dp)) {
            ComposerBox(
                model, draft, if (working) "Tell $name while it works" else "Tell $name more", room * 0.3f,
                account = choice?.let { model.account(it.harness, it.model) },
                harness = choice?.harness ?: model.models?.harness,
                agents = AgentScope.OfThread(id),
                pickers = { at ->
                    val own = choice
                    if (own != null) Pickers(model, at, worker = own, choose = choose) else Spacer(at)
                },
            ) { empty ->
                if (working && empty) {
                    StopButton(enabled = !stopping) {
                        stopping = true
                        model.act { model.host?.agentStop(id) }
                    }
                } else {
                    SendButton(enabled = !empty) {
                        model.tell(id, draft.text.trim(), draft.files.toList())
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
}

/** A thread's log as the chat's messages (App.tsx logMessages), so it is
 * drawn as the chat is: its task and the person's words as theirs, each run
 * of calls and thinking with the words that end it as one answer, words
 * before a call as words on the way, a file as a file. The live run is the
 * last answer while it works; `telling`, words not yet in the log, wait
 * their turn under it. `rich` holds the calls the stream told in full. */
fun logMessages(all: List<Step>, telling: List<Telling>, working: Boolean, rich: List<Message>, detail: String): List<Message> {
    val tools = rich.flatMap { it.tools }
    val out = mutableListOf<Message>()
    fun message(id: String, role: Role, text: String = "", tools: List<ToolCall> = emptyList(), thinking: String = "", file: String? = null, picture: String? = null, waits: Boolean = false) = Message(
        id = id, role = role, text = text, thinking = thinking, progress = false, metrics = null, tools = tools, live = false, plain = false,
        picture = picture, file = file, files = emptyList(), reaction = null, outbox = Outbox.ARRIVED, waitsTurn = waits, date = null,
        segment = null, woke = null, quote = null, from = null, row = null, keys = emptyList(),
    )
    // The answer the calls since the last words go on, by its place in `out`.
    var answer = -1
    all.forEachIndexed { i, s ->
        val id = "s/$i"
        when (s.kind) {
            StepKind.TASK, StepKind.TOLD -> { answer = -1; out += message(id, Role.USER, personWords(s.text)) }
            StepKind.FILE -> {
                answer = -1
                out += if (PICTURE.containsMatchIn(s.text)) message(id, Role.ASSISTANT, picture = s.text) else message(id, Role.ASSISTANT, file = s.text)
            }
            StepKind.WORDS -> {
                // Words after calls end their answer; words before one are on the way.
                val on = all.drop(i + 1).takeWhile { it.kind != StepKind.TOLD && it.kind != StepKind.TASK }.any { it.kind == StepKind.CALL }
                if (answer >= 0 && out[answer].text.isEmpty()) out[answer] = out[answer].copy(text = s.text, progress = on)
                else out += message(id, Role.ASSISTANT, s.text).copy(progress = on)
                answer = -1
            }
            StepKind.CALL -> {
                val call = tools.lastOrNull { t -> s.id != null && t.id == s.id }
                    ?: ToolCall(id = s.id ?: s.name, name = s.name, input = s.text, arguments = s.arguments, result = s.result, done = s.result != null, error = false, structured = null, thought = null, report = null, heard = false)
                if (answer < 0) { out += message(id, Role.ASSISTANT); answer = out.lastIndex }
                out[answer] = out[answer].copy(tools = out[answer].tools + call)
            }
            // A scratchpad's thinking shows only while its run is at it.
            StepKind.THOUGHT -> if (working && i == all.lastIndex) {
                if (answer < 0) { out += message(id, Role.ASSISTANT); answer = out.lastIndex }
                out[answer] = out[answer].copy(thinking = s.text)
            }
        }
    }
    val end = out.indexOfLast { it.role == Role.ASSISTANT }
    if (end >= 0 && end == out.lastIndex) {
        val metrics = if (detail == "full") rich.lastOrNull { it.metrics != null }?.metrics else null
        out[end] = out[end].copy(live = working, metrics = metrics)
    }
    telling.forEach { out += message("t/${it.id}", Role.USER, it.text, waits = true) }
    return out
}

private val PICTURE = Regex("\\.(jpe?g|png|gif|webp)$", RegexOption.IGNORE_CASE)
