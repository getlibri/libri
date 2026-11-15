package su.libri

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntSize
import kotlinx.coroutines.delay

// What the person opens in a chat stays in view (reveal.ts; the person
// asked that something opened off screen scroll down until its top fits):
// opening a row, a fold or a card
// scrolls the talk so the opened block shows whole, or, taller than the
// view, so its top sits just under the chat's head. Only the person's tap
// moves the view, never a turn streaming on its own.

/** The person's taps on a fold: each opening one counts in `count`, and the
 * block comes into view; each closing one counts in `shut`, with where the
 * block's top stood, which it keeps. */
class Opened {
    var count by mutableIntStateOf(0)
        private set
    var shut by mutableIntStateOf(0)
        private set
    internal var top = 0f
    internal var at = 0f

    fun tap() {
        count++
    }

    /** A tap that closes it: the block's top stays where it stands now. */
    fun close() {
        at = top
        shut++
    }
}

/** The talk's list, which a closing fold scrolls to keep its row in place. */
val LocalTalkList = compositionLocalOf<LazyListState?> { null }

/** Brings the block into view each time `opened` counts a tap, with the
 * head's room above it (the status bar and the floating buttons), and
 * again as it grows over the next moment (a fold that animates open, a card
 * that loads). */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Modifier.revealed(opened: Opened): Modifier {
    val requester = remember { BringIntoViewRequester() }
    var size by remember { mutableStateOf(IntSize.Zero) }
    val room = with(LocalDensity.current) { (statusTop + CHROME_TOP).toPx() }
    LaunchedEffect(opened.count) {
        if (opened.count == 0) return@LaunchedEffect
        // The head's room is asked for too, so the block's top lands under it.
        repeat(6) {
            delay(60)
            requester.bringIntoView(Rect(0f, -room, size.width.toFloat(), size.height.toFloat()))
        }
    }
    // Closed, the row stays put: the talk runs newest at its foot
    // (reverseLayout), so what shrinks above pulls the rest down; the list
    // is scrolled back, frame by frame while the fold settles.
    val list = LocalTalkList.current
    LaunchedEffect(opened.shut) {
        if (opened.shut == 0 || list == null) return@LaunchedEffect
        val until = System.nanoTime() + 400_000_000L
        while (System.nanoTime() < until) {
            withFrameNanos { }
            val by = opened.top - opened.at
            if (kotlin.math.abs(by) > 1f) list.scrollBy(-by)
        }
    }
    return this.onSizeChanged { size = it }.onGloballyPositioned { opened.top = it.positionInWindow().y }.bringIntoViewRequester(requester)
}
