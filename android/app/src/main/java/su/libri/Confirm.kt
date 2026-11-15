package su.libri

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog

/** Libri's own confirm (as the web's dialog.tsx): the paper, type and
 * buttons of the app, not the platform's alert. `confirm` names what it
 * does ("Merge", "Revoke"); `danger` puts it in the accent colour for what
 * can't be undone. Back or a tap outside cancels. */
@Composable
fun ConfirmDialog(
    title: String,
    body: String? = null,
    confirm: String,
    danger: Boolean = false,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    val p = Ink
    Dialog(onDismissRequest = onDismiss) {
        Column(
            Modifier.widthIn(max = 400.dp).fillMaxWidth()
                .shadow(12.dp, RoundedCornerShape(16.dp))
                .clip(RoundedCornerShape(16.dp))
                .background(p.paper)
                .border(1.25.dp, p.edge, RoundedCornerShape(16.dp))
                .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, style = type(TypeScale.body, 1.35f, FontWeight.Medium), color = p.ink)
            body?.let { Text(it, style = type(TypeScale.label, 1.45f), color = p.mid) }
            Spacer(Modifier.height(4.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, androidx.compose.ui.Alignment.End)) {
                Text(
                    "Cancel",
                    style = type(TypeScale.label, 1.3f),
                    color = p.mid,
                    modifier = Modifier.clip(RoundedCornerShape(50)).clickable(onClick = onDismiss).padding(horizontal = 16.dp, vertical = 8.dp),
                )
                Text(
                    confirm,
                    style = type(TypeScale.label, 1.3f),
                    color = p.paper,
                    modifier = Modifier.clip(RoundedCornerShape(50)).background(if (danger) p.accent else p.ink)
                        .clickable(onClick = onConfirm).padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
        }
    }
}
