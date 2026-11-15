package su.libri

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Keeps a reversed talk on its newest, as the page's thread follows its foot
 * (assistant-ui's useThreadViewportAutoScroll, `stuck` in AgentPage): pinned
 * until the person scrolls away from the foot, and again once they come back
 * to it or send something.
 *
 * A LazyColumn holds its place by key, so an item added at index 0 (a message,
 * the dots) lands below the viewport unless the list is told to go to it. At
 * index 0 and offset 0 the foot stands at the viewport's foot whatever else
 * changes, so the composer growing and the keyboard rising need nothing here.
 *
 * `holding` keeps the room a turn has taken, as the page's useFootHold does:
 * hung from the foot, the talk fell whenever a tool row ended or the dots
 * left, and rose again with the next, a bounce. Pinned, what the foot gives
 * back goes into a blank item under it ([foot]), which gives it up again as
 * the talk grows into it, so nothing in view moves. The person scrolling up
 * past the blank lets it go, so coming back down finds the talk's own foot.
 *
 * A finished turn releases its room. Keeping that transient height across
 * history replacement or return left blank space beneath the final reply.
 *
 * Only a turn's room is held: until a turn has run since the list began (or
 * was reset), a fall is the old talk settling, as on opening the app again,
 * and holding it left a screen's height of blank under the last line.
 */
class Follow(private val holding: Boolean = false) {
    var pinned by mutableStateOf(true)

    /** The blank's height in px; read in its layout, so state. */
    private var room by mutableIntStateOf(0)

    /** Lets go of the blank. */
    fun reset() {
        held = null
        room = 0
        armed = false
    }

    // Whether a turn has run since the list began or was reset (Keep's `live`).
    private var armed = false

    /**
     * The blank, put first in a holding list so it hangs at the very foot. The
     * list spaces items apart even when this one is 0 tall; the list's foot
     * padding is that much less to make up for it.
     */
    fun foot(scope: LazyListScope) = scope.item(key = FOOT) {
        Box(Modifier.layout { m, k ->
            val p = m.measure(k.copy(minHeight = 0, maxHeight = 0))
            layout(p.width, room) {}
        })
    }

    // A scroll towards the older messages lets go of the foot. What a scroll
    // moves the held edge by is the person's, so it moves the mark too: the
    // drift is then only what the talk itself did, even mid-scroll.
    private val gesture = object : NestedScrollConnection {
        override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset {
            if (available.y > 0) pinned = false
            byHand = true
            return Offset.Zero
        }

        override suspend fun onPostFling(consumed: Velocity, available: Velocity): Velocity {
            byHand = false
            return Velocity.Zero
        }

        override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
            // Down the screen is towards the foot, which the edge is measured from.
            held = held?.let { it.copy(edge = it.edge - consumed.y) }
            return Offset.Zero
        }
    }

    // The oldest item in view, how far its top edge stood from the list's
    // foot in the last layout, and the list's height then. Read and written
    // in layout and in the scroll, so a plain field rather than state.
    private data class Mark(val key: Any, val edge: Float, val height: Int)

    private var held: Mark? = null

    // A drag or its fling is under way: the scroll is the person's, which the
    // mark follows. Any other scroll (a quote's jump) is the page's own, and
    // the drift waits for it to end.
    private var byHand = false

    /**
     * How far the held top edge has risen in the layout just measured, by what
     * the talk did rather than a scroll. Measured while a finger holds the
     * list or a fling runs too: skipped then, the reply growing under a
     * finger resting on the text moved what it was reading.
     */
    private fun drift(list: LazyListState): Int {
        val b = held ?: return 0
        if ((pinned && !holding) || (list.isScrollInProgress && !byHand)) return 0
        val info = list.layoutInfo
        val now = info.visibleItemsInfo.firstOrNull { it.key == b.key } ?: return 0
        // From the foot, pinned: a dock or keyboard rising under the talk
        // lifts it with the foot, as it should. Scrolled up, from the top: the
        // list hangs from its foot, so a dock growing under it (a question,
        // a task) lifted what the person read by as much, as the page's dock
        // once scrolled it.
        val shrank = if (pinned) 0 else b.height - info.viewportSize.height
        val rise = (now.offset + now.size - b.edge).roundToInt() + shrank
        // A scroll's fractions round to a pixel either way.
        if (abs(rise) <= 1) return 0
        // Pinned, a fall is made up by the blank growing, and a rise only as
        // far as the blank can give back; past that the talk rises as it should.
        if (pinned && rise < 0 && !armed) return 0
        return if (!pinned || rise < 0) rise else minOf(rise, room)
    }

    /**
     * Marks afresh, once the layout stands still, the oldest item whose top is
     * in view (else the one filling it): one cut at the top would leave the
     * view as a dock rises under it, and with it the mark.
     */
    private fun mark(list: LazyListState) {
        val info = list.layoutInfo
        val seen = info.visibleItemsInfo
        val item = seen.lastOrNull { it.offset + it.size <= info.viewportEndOffset } ?: seen.lastOrNull()
        held = item?.let { Mark(it.key, (it.offset + it.size).toFloat(), info.viewportSize.height) }
    }

    /**
     * Put on the list. Keep's effect puts a drift back on the scroll offset,
     * but only after the frame that grew has been drawn, so each step of a
     * streaming reply would show for a frame. Until then the list is drawn
     * lowered by the drift, where the scroll is about to put it; that scroll
     * remeasures the list at once, and the drift reads 0 again. This goes if
     * LazyList comes to anchor a reversed list on its last visible item.
     */
    fun on(list: LazyListState): Modifier = Modifier.nestedScroll(gesture).layout { measurable, constraints ->
        val placeable = measurable.measure(constraints)
        val drift = Snapshot.withoutReadObservation {
            drift(list).also { if (it == 0) mark(list) }
        }
        layout(placeable.width, placeable.height) { placeable.place(0, drift) }
    }

    @Composable
    fun Keep(list: LazyListState, live: Boolean) {
        LaunchedEffect(list) {
            reset()
            if (pinned) list.requestScrollToItem(0)
        }
        LaunchedEffect(list, live) {
            if (!live) {
                room = 0
                armed = false
                held = null
                if (pinned) list.requestScrollToItem(0)
            }
        }
        if (live) SideEffect { armed = true }
        if (holding) {
            Release(list)
            Cap(list)
        }
        // A drag let go without a fling may end with no onPostFling.
        LaunchedEffect(list) {
            snapshotFlow { list.isScrollInProgress }.collect { if (!it) byHand = false }
        }
        // The page's own distance for being at the bottom.
        val near = with(LocalDensity.current) { 8.dp.toPx() }
        LaunchedEffect(list) {
            snapshotFlow { !pinned && list.atFoot(near) }.collect { if (it) pinned = true }
        }
        LaunchedEffect(list) {
            snapshotFlow { pinned && !list.isScrollInProgress && !list.atFoot(0f) }
                .collect { if (it) list.requestScrollToItem(0) }
        }
        // A reversed list hangs its items from the foot, so whatever grows or
        // arrives there (the reply streaming in, a new step, with the dots or
        // the jobs row still under it) lifts everything above it. Once the
        // person has scrolled away, the top edge of the oldest item in view
        // holds its place: what it moved by between two layouts that no scroll
        // made goes back on the offset. Its top, not its offset: the reply
        // itself may be that item, taller than the screen and read from its
        // start, and its foot doesn't move as it grows. An item's offset
        // counts from the list's start, its foot when reversed.
        // The mark is taken in layout (on), where no scroll can come between
        // reading the edge and keeping it.
        LaunchedEffect(list) {
            snapshotFlow { list.layoutInfo.let { i -> i.viewportSize.height to i.visibleItemsInfo.map { it.key to it.offset + it.size } } }
                .collect {
                    val drift = drift(list)
                    if (drift == 0) return@collect
                    if (pinned) room = (room - drift).coerceAtLeast(0) else list.dispatchRawDelta(drift.toFloat())
                }
        }
    }

    @Composable
    private fun Release(list: LazyListState) {
        // Scrolled up, the blank goes as it leaves the view below: whole once
        // the list stands on an item above it, which nothing in view hangs
        // from; in part once a scroll that stopped inside it ends, cutting
        // what is out of view and moving the scroll by the same, so what is
        // in view stays where it is.
        LaunchedEffect(list) {
            snapshotFlow { Triple(pinned, list.isScrollInProgress, list.firstVisibleItemIndex to list.firstVisibleItemScrollOffset) }
                .collect { (pinned, scrolling, at) ->
                    val (index, offset) = at
                    if (pinned || room == 0) return@collect
                    if (index > 0) room = 0
                    else if (!scrolling && offset > 0) {
                        val cut = minOf(offset, room)
                        room -= cut
                        list.requestScrollToItem(0, offset - cut)
                    }
                }
        }
    }

    /**
     * The blank holds no more than lets the person's last message stand at
     * the top of the view, under the header the list's top padding clears,
     * as the page's useFootHold caps it. Uncapped, a short question whose
     * answer gave back room mid-turn stood up under the floating header
     * (Temp's Memory and Clear pills), above the line the talk starts on.
     */
    @Composable
    private fun Cap(list: LazyListState) {
        LaunchedEffect(list) {
            snapshotFlow {
                val info = list.layoutInfo
                val asked = info.visibleItemsInfo.firstOrNull { it.contentType == ASKED }
                // Reversed, an item's offset counts up from the foot; the
                // talk's first line stands below the top padding.
                if (asked == null || room == 0) 0 else asked.offset + asked.size - (info.viewportEndOffset - info.afterContentPadding)
            }.collect { over ->
                if (over <= 0 || !pinned) return@collect
                val cut = minOf(over, room)
                room -= cut
                // The talk falls by the cut, which is no drift to make up.
                held = held?.let { it.copy(edge = it.edge - cut) }
            }
        }
    }

    private fun LazyListState.atFoot(near: Float) = firstVisibleItemIndex == 0 && firstVisibleItemScrollOffset <= near

    companion object {
        /** The contentType of an item holding the person's own words. */
        const val ASKED = "asked"
        private const val FOOT = "foot"
    }
}
