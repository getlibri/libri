package su.libri

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import su.libri.core.ReplyOffer

/** The replies the newest message offers, as a row of buttons under it, as
 * a messaging app's suggested replies (a design choice of 2026-11-06: "quick-reply chips
 * under the relevant assistant message, not above composer"). A tap sends
 * the reply tied to the message, or answers the question asked; the row
 * goes once the person answers or anything later is said (`offered` in the
 * core). App.tsx `Replies` is the page's. */
@Composable
fun Replies(offer: ReplyOffer, tap: (String) -> Unit) {
    val p = Ink
    var sent by remember(offer) { mutableStateOf<String?>(null) }
    Column(
        Modifier.padding(start = 4.dp).semantics { contentDescription = offer.question ?: "Suggested replies" },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
    // A question's chips carry their question, or they answer nothing seen.
    offer.question?.let { Text(it, style = type(TypeScale.body, 1.4f, FontWeight.Medium), color = p.ink) }
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        itemVerticalAlignment = Alignment.CenterVertically,
    ) {
        offer.replies.forEach { r ->
            val shape = RoundedCornerShape(50)
            // The pill is the target, 44dp tall as the phone page's chips
            // (spec decision 4 pending); a wrapped box inside a taller
            // slot left only the drawn pill tappable (2026-11-06).
            Box(
                Modifier.alpha(if (sent == null) 1f else 0.5f).clip(shape).background(p.lift)
                    .border(1.dp, p.frame, shape).clickable(enabled = sent == null) { sent = r; tap(r) }
                    .heightIn(min = 44.dp).padding(horizontal = 14.dp, vertical = 8.dp),
                contentAlignment = Alignment.CenterStart,
            ) { Text(r, style = type(TypeScale.control, 1.35f), color = p.ink) }
        }
    }
    }
}
