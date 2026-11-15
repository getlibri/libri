package su.libri

// The agents running, at the composer (agents.tsx): on the composer's whole
// outline, at its top right, sit opaque overlapping circles, one per worker at
// work, each with its model's mark. A tap opens the tree of them upward out of
// the composer; a row opens that worker's thread as a tab of its own.
//
// The app draws workers alone: it reads /api/work, not the chat stream the
// web reads a harness's own subagents from (WorkTree.kt leaves them out too).

import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sign
import kotlin.math.sin
import androidx.compose.animation.core.Easing
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.draw.drawBehind
import androidx.compose.foundation.background
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.first
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.positionOnScreen
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.zIndex
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import su.libri.core.PhonePane
import su.libri.core.Thread

/** Whose agents a composer shows: every one in the main chat; in a worker's
 * thread, its own sub-workers. */
sealed interface AgentScope {
    data object Main : AgentScope
    data class OfThread(val id: String) : AgentScope
}

/** How far in from the composer's right the circles sit: clear of its
 * rounded corner. */
val AGENTS_END = 30.dp

private class Agent(val t: Thread, val act: Act, val logo: Int?, val kids: List<Agent>)

/** How many circles show before the rest go under a "+N". */
/** A circle's width, and how far each next one steps on over it. */
private val CIRCLE = 22.dp
private val STEP = 15.dp

/** A worker counts as running while it works or asks the person for an
 * approval or an answer. One done (its report waiting to be read), napping,
 * asleep or archived leaves the circles and the column. */
private fun Thread.running() = state == "working" || asks(this)

private fun agentsOf(threads: List<Thread>, scope: AgentScope): List<Agent> {
    val byId = threads.associateBy { it.id }
    // A running worker hangs from its nearest running head, so one asked for
    // by a worker now asleep still shows, at the top.
    fun headOf(t: Thread): String? {
        var h = t.head
        while (h != null) {
            val head = byId[h] ?: return null
            if (head.running()) return h
            h = head.head
        }
        return null
    }
    val live = threads.filter { it.running() }
    fun agent(t: Thread): Agent = Agent(
        t, actOf(t), t.pick?.let { logoOf(it.model, "", it.harness) },
        live.filter { headOf(it) == t.id }.map(::agent),
    )
    return when (scope) {
        AgentScope.Main -> live.filter { headOf(it) == null }.map(::agent)
        is AgentScope.OfThread -> live.filter { headOf(it) == scope.id }.map(::agent)
    }
}

private fun flat(list: List<Agent>): List<Agent> = list.flatMap { listOf(it) + flat(it.kids) }

/** The ring for what an agent is doing: the person's amber for a wait on
 * them, red for a failure, none while it simply works. */
private fun ringOf(act: Act): Color? = if (act == Act.PERSON || act == Act.FAILED) act.colour else null

/** The circles in the composer's outline, laid over its top edge, and the
 * tree they open. Draws nothing while no agent runs. */
@Composable
fun ComposerAgents(model: ChatModel, scope: AgentScope, width: Int, modifier: Modifier = Modifier) {
    val p = Ink
    val agents = agentsOf(model.threads, scope)
    val all = flat(agents)
    var open by remember { mutableStateOf(false) }
    LaunchedEffect(all.isEmpty()) { if (all.isEmpty()) open = false }
    if (all.isEmpty()) return
    // The row grows leftward, one circle per agent, until it stands as far
    // from the composer's left end as from its right; only past that does
    // its last circle become "+N".
    val density = LocalDensity.current
    val room = with(density) { maxOf(1, ((width - 2 * AGENTS_END.toPx() - CIRCLE.toPx()) / STEP.toPx()).toInt() + 1) }
    val slots = if (all.size <= room) all.size else maxOf(1, room - 1)
    // Those waiting on the person come first, so a "+N" never hides one.
    // On the bar they stand in the column's order reversed, so the one
    // rising highest leads the train along the rail and none cross.
    val shown = all.sortedBy { if (it.act == Act.PERSON) 0 else if (it.act == Act.FAILED) 1 else 2 }.take(slots).sortedByDescending { all.indexOf(it) }
    val more = all.size - shown.size
    val waiting = all.count { it.act == Act.PERSON }
    val label = "${all.size} agent${if (all.size == 1) "" else "s"} running" + if (waiting > 0) ", $waiting waiting on you" else ""
    // Where each circle sits in the window, by its worker ("+" for the
    // "+N"), for the column's dots to rise from and go back to.
    val circles = remember { mutableStateMapOf<String, Offset>() }
    // Folding: the dots travel back to the row, then the column goes.
    var leaving by remember { mutableStateOf(false) }
    // Whether the column's dots stand over the circles yet: the popup is a
    // window of its own, laid out a few frames after the tap, and hiding
    // the circles before then left the bar empty for those frames.
    var flying by remember { mutableStateOf(false) }
    LaunchedEffect(open) { if (!open) flying = false }
    val fold = { leaving = true }
    Box(modifier) {
        // Out in the column, these circles are the dots there: hidden here
        // once the dots stand over them, until they are home (agents.tsx).
        Row(
            Modifier.clip(CircleShape).graphicsLayer { alpha = if (open && flying) 0f else 1f }.clickable { if (open) fold() else open = true }.semantics { contentDescription = label },
            verticalAlignment = Alignment.CenterVertically,
            // Overlapping, as the web's circles do.
            horizontalArrangement = Arrangement.spacedBy((-7).dp),
        ) {
            shown.forEach { a -> Circle(a, Modifier.onGloballyPositioned { circles[a.t.id] = it.centerOnScreen() }) }
            if (more > 0) Box(
                Modifier.size(22.dp).clip(CircleShape).background(p.lift).padding(2.dp).clip(CircleShape).background(p.hover)
                    .onGloballyPositioned { circles["+"] = it.centerOnScreen() },
                contentAlignment = Alignment.Center,
            ) { Text("+$more", style = type(10f, 1f, FontWeight.SemiBold), color = p.mid) }
        }
        // The whole window, so the talk behind dims; the column stands over
        // the row's last circle, rising from just above it.
        if (open) Popup(
            popupPositionProvider = WindowOrigin,
            onDismissRequest = fold,
            properties = PopupProperties(focusable = true),
        ) {
            val last = shown.lastOrNull()?.let { circles[if (more > 0) "+" else it.t.id] }
            AgentColumn(
                agents, model.models?.offers.orEmpty(), last, circles, leaving, fold, { open = false; leaving = false }, { flying = true },
                all = if (scope == AgentScope.Main) ({ open = false; model.allWorkersAsked++ }) else null,
                start = if (scope == AgentScope.Main) ({ open = false; model.newTask = true }) else null,
            ) { t ->
                open = false
                if (t.state == "waiting") model.workAct(t.id, "read")
                model.tabAsked = PhonePane("thread", t.id)
            }
        }
    }
}

/** The longest dot's travel between the row and the column, inside the
 * 400 ms every interaction is held to; the others take time in step with
 * their shorter way, so all keep one speed (agents.tsx TRAVEL). */
private const val TRAVEL_MS = 200
private const val TRAVEL_LEAST_MS = 86
/** The rail's corner, from the bar up into the column (agents.tsx BEND). */
private val BEND = 20.dp
/** The share of a dot's time it spends gathering speed, and as much
 * slowing at the end; between, its speed holds (agents.tsx RAMP). */
private const val RAMP = 0.22f

/** How far along its rail a dot is at time `t` (both 0 to 1): speed rises
 * evenly for `RAMP`, holds, and falls as evenly to rest (agents.tsx along). */
private val Along = Easing { t ->
    val top = 1 / (1 - RAMP)
    when {
        t < RAMP -> top * t * t / (2 * RAMP)
        t > 1 - RAMP -> 1 - top * (1 - t) * (1 - t) / (2 * RAMP)
        else -> top * (t - RAMP / 2)
    }
}

/** The rail from a bar circle to a dot's slot: along the bar, a quarter
 * circle of `bend` (less where the way is shorter) and up; `at(s)` is the
 * point `s` px along it (agents.tsx rail). */
private class Rail(val from: Offset, val to: Offset, bend: Float) {
    private val h = sign(to.x - from.x)
    private val r = minOf(bend, abs(to.x - from.x), maxOf(0f, from.y - to.y))
    private val run = abs(to.x - from.x) - r
    private val turn = (PI / 2).toFloat() * r
    private val corner = Offset(to.x - h * r, from.y - r)
    val length = run + turn + maxOf(0f, from.y - to.y) - r
    fun at(s: Float): Offset = when {
        s <= run -> Offset(from.x + h * s, from.y)
        s <= run + turn -> { val a = if (r > 0f) (s - run) / r else 0f; Offset(corner.x + h * r * sin(a), corner.y + r * cos(a)) }
        else -> Offset(to.x, maxOf(to.y, from.y - r - (s - run - turn)))
    }
}

/** A layout's centre on the screen, the one space the app's window and a
 * popup's share. */
private fun LayoutCoordinates.centerOnScreen() = positionOnScreen() + Offset(size.width / 2f, size.height / 2f)

/** A column dot at rest: no flight under way and out in its slot. */
private fun landed(t: Animatable<Float, AnimationVector1D>) = !t.isRunning && t.value == 1f

/** An agent's dot in the column, 28dp, its logo on the lifted fill. */
@Composable
private fun AgentDot(a: Agent, modifier: Modifier = Modifier) {
    val p = Ink
    val ring = ringOf(a.act)
    Box(
        modifier.size(28.dp).clip(CircleShape).background(ring ?: p.lift).padding(if (ring != null) 1.5.dp else 2.dp)
            .clip(CircleShape).background(p.hover),
        contentAlignment = Alignment.Center,
    ) { CompositionLocalProvider(LocalContentColor provides p.mid) { Logo(a.logo, 14.dp) } }
}

@Composable
private fun Circle(a: Agent, modifier: Modifier = Modifier) {
    val p = Ink
    val ring = ringOf(a.act)
    Box(
        // A ringed one draws over its neighbour, so its ring shows whole.
        modifier.then(if (ring != null) Modifier.zIndex(1f) else Modifier).size(22.dp).clip(CircleShape).background(ring ?: p.lift).padding(if (ring != null) 1.5.dp else 2.dp)
            .clip(CircleShape).background(p.lift).padding(if (ring != null) 0.5.dp else 0.dp).clip(CircleShape).background(p.hover),
        contentAlignment = Alignment.Center,
    ) { CompositionLocalProvider(LocalContentColor provides p.mid) { Logo(a.logo, 12.dp) } }
}

/** The agents risen out of the composer as a column, no box around them
 * (agents.tsx AgentTree): each dot travels from its circle to its slot, one
 * under another above the composer's edge, its name, model and state to its
 * left, the talk behind dimmed; a sub-worker stands in under its head. */
@Composable
private fun AgentColumn(
    agents: List<Agent>,
    offers: List<su.libri.core.Offer>,
    over: Offset?,
    circles: Map<String, Offset>,
    leaving: Boolean,
    fold: () -> Unit,
    folded: () -> Unit,
    placed: () -> Unit,
    all: (() -> Unit)?,
    start: (() -> Unit)?,
    open: (Thread) -> Unit,
) {
    val p = Ink
    val rows = mutableListOf<Pair<Agent, Int>>()
    fun walk(list: List<Agent>, d: Int) {
        list.forEach { rows += it to d; walk(it.kids, d + 1) }
    }
    walk(agents, 0)
    val context = LocalContext.current
    val still = remember { android.provider.Settings.Global.getFloat(context.contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
    val dim = remember { Animatable(if (still) 1f else 0f) }
    LaunchedEffect(leaving) { dim.animateTo(if (leaving) 0f else 1f, tween(200)) }
    val density = LocalDensity.current
    // Each dot's time out of the row (0 on the bar, 1 in its slot) and its
    // slot in the window, shared by the column and the flight above it.
    val times = remember { mutableMapOf<String, Animatable<Float, AnimationVector1D>>() }
    fun timeOf(id: String) = times.getOrPut(id) { Animatable(if (still) 1f else 0f) }
    val slots = remember { mutableStateMapOf<String, Offset>() }
    // Where the column's window starts on the screen; none until laid out.
    var origin by remember { mutableStateOf<Offset?>(null) }
    // Folding ends when every dot is home on the bar, not after a set
    // time: a timer is not stretched by the system's animation scale, and
    // at 10x it closed the column with the dots barely set off.
    // The rows there now: a worker done mid-fold has no dot to wait for.
    val home by rememberUpdatedState(rows.map { it.first.t.id })
    LaunchedEffect(leaving) {
        if (!leaving) return@LaunchedEffect
        if (!still) snapshotFlow { home.all { id -> timeOf(id).let { !it.isRunning && it.value == 0f } } }.first { it }
        folded()
    }
    BoxWithConstraints(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionOnScreen() }) {
        // The talk behind, dimmed as the web dims it (agents.tsx, 70%).
        Box(Modifier.fillMaxSize().graphicsLayer { alpha = dim.value }.background(p.paper.copy(alpha = 0.7f)).pointerInput(Unit) { detectTapGestures { fold() } })
        // The bar's circles are measured in the app's window, this column in
        // the popup's, and the two windows do not start at the same place on
        // the screen: everything is measured on the screen, then made the
        // popup's by its origin, or the column and the dots stood a circle
        // and more low and to the right of the bar.
        val zero = origin ?: return@BoxWithConstraints
        LaunchedEffect(Unit) { placed() }
        val bar = over ?: return@BoxWithConstraints
        val at = bar - zero
        // To the composer's right end (AGENTS_END and half a circle past
        // the bar's last one), as on the web: the dots stand right of that
        // circle, room for the rail's corner.
        val end = with(density) { (constraints.maxWidth - at.x - AGENTS_END.toPx() - (CIRCLE / 2).toPx()).coerceAtLeast(0f).toDp() }
        val bottom = with(density) { (constraints.maxHeight - at.y + 11.dp.toPx() + 8.dp.toPx()).coerceAtLeast(0f).toDp() }
        // On the surface every floating panel has, fading in under the dots
        // as they rise: with the talk showing through, the rows were hard
        // to read (the web's data-agents-panel).
        Column(
            Modifier.align(Alignment.BottomEnd).padding(end = end, bottom = bottom).widthIn(max = maxWidth - 28.dp).heightIn(max = maxHeight * 0.6f)
                .drawBehind {
                    if (dim.value <= 0f) return@drawBehind
                    val r = CornerRadius(Radii.xxl.toPx())
                    drawRoundRect(p.lift.copy(alpha = dim.value), cornerRadius = r)
                    drawRoundRect(p.hair.copy(alpha = dim.value), cornerRadius = r, style = Stroke(1.dp.toPx()))
                }
                .verticalScroll(rememberScrollState())
                .padding(start = 8.dp, end = 8.dp, top = 8.dp),
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            // All set off together and keep one speed, so they leave the bar
            // as a train, each turning off where its row is; folding is the
            // same run played backwards, the last to leave the first home.
            // Each waits for every slot to be laid out, to know the longest way.
            val bend = with(density) { BEND.toPx() }
            fun railOf(id: String) = slots[id]?.let { Rail(circles[id] ?: circles["+"] ?: bar, it, bend) }
            rows.forEach { (a, d) ->
                val t = timeOf(a.t.id)
                LaunchedEffect(leaving) {
                    if (still) return@LaunchedEffect
                    snapshotFlow { home.all { slots[it] != null } }.first { it }
                    val longest = maxOf(1f, home.maxOf { railOf(it)?.length ?: 0f })
                    val time = maxOf(TRAVEL_LEAST_MS, (TRAVEL_MS * (railOf(a.t.id)?.length ?: 0f) / longest).toInt())
                    // The delay is the tween's own, so the system's animation
                    // scale stretches it with the rest.
                    if (leaving) t.animateTo(0f, tween(time, TRAVEL_MS - time, Along))
                    else t.animateTo(1f, tween(time, 0, Along))
                }
                Row(
                    Modifier.padding(end = (d * 18).dp).clip(CircleShape).clickable { open(a.t) }.padding(start = 12.dp, top = 2.dp, bottom = 2.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Column(Modifier.weight(1f, fill = false).graphicsLayer { alpha = t.value.coerceIn(0f, 1f); translationX = (1 - t.value.coerceIn(0f, 1f)) * 8.dp.toPx() }, horizontalAlignment = Alignment.End) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            shortModel(a.t, offers)?.let { Text(it, style = type(11f, 1.3f, family = Faces.mono), color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                            Text(a.t.name, style = type(TypeScale.caption, 1.3f, FontWeight.SemiBold), color = p.ink, maxLines = 1)
                        }
                        val status = a.t.summary ?: a.t.title ?: a.t.brief.lineSequence().firstOrNull()
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(ago(a.t.at), style = type(TypeScale.caption, 1.3f), color = p.soft)
                            if (!status.isNullOrBlank()) Text(status, style = type(TypeScale.caption, 1.3f), color = p.soft, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                            Box(Modifier.size(6.dp).clip(CircleShape).background(a.act.colour ?: p.soft))
                        }
                    }
                    // Its slot: the dot shows here once landed, so it scrolls
                    // and clips with the column; in flight it is drawn above.
                    AgentDot(a, Modifier.onGloballyPositioned { slots[a.t.id] = it.centerOnScreen() }
                        .graphicsLayer { alpha = if (landed(t)) 1f else 0f })
                }
            }
            // Every worker, asleep and archived too, with the watches (the Work
            // page laid over), and starting one.
            // Its words end where the rows' do, clear of the dots' rail.
            if (all != null || start != null) Row(Modifier.padding(end = 26.dp).graphicsLayer { alpha = dim.value }, verticalAlignment = Alignment.CenterVertically) {
                all?.let { Text("Show all", style = type(TypeScale.caption, 1.3f), color = p.mid, modifier = Modifier.clip(CircleShape).clickable(onClick = it).padding(horizontal = 12.dp, vertical = 8.dp)) }
                start?.let { Text("Start a worker", style = type(TypeScale.caption, 1.3f), color = p.mid, modifier = Modifier.clip(CircleShape).clickable(onClick = it).padding(horizontal = 12.dp, vertical = 8.dp)) }
            }
        }
        // The dots in flight, over the bar and the column alike: drawn in
        // their rows, the rows' clip and the column's scroll cut them off,
        // so each showed only growing out of its own row (the web flies
        // them in a portal, agents.tsx).
        rows.forEach { (a, _) ->
            val t = timeOf(a.t.id)
            val from = circles[a.t.id] ?: circles["+"] ?: bar
            AgentDot(a, Modifier.graphicsLayer {
                if (landed(t)) { alpha = 0f; return@graphicsLayer }
                val there = slots[a.t.id] ?: from
                val rail = Rail(from, there, BEND.toPx())
                val u = t.value
                val now = rail.at(u * rail.length)
                val half = 14.dp.toPx()
                translationX = now.x - zero.x - half
                translationY = now.y - zero.y - half
                // The bar's circle is 22dp, the column's dot 28dp.
                val small = 22f / 28f
                scaleX = small + (1 - small) * u
                scaleY = scaleX
            })
        }
    }
}
