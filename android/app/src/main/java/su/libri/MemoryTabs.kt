package su.libri

// A chat item's memories as tabs along the top of the Chat page
// (memoryTabs.tsx; tiles.rs: the chat kind keeps `tabs`): Main and each
// named memory opened in it, the one shown its `arg`. The + offers the
// memories not open here and a new one. Closing a tab only takes it from the
// item; the memory and its talk stay.

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import su.libri.core.PhonePane

/** A chat pane's tabs: its own, else the one memory it shows. */
fun PhonePane.memoryTabs(): List<String> = tabs?.takeIf { it.isNotEmpty() } ?: listOf(arg.orEmpty())

/** The chat pane showing `id`, its tabs `tabs` (none kept for one alone). */
fun chatPane(id: String, tabs: List<String>) =
    PhonePane("chat", id.ifEmpty { null }, if (tabs.size > 1 || tabs.firstOrNull() != id) tabs else null)

/** The tab row: `show` puts the item on that memory, opening its tab if it
 * has none; `set` keeps a pane as it is. */
@Composable
fun MemoryTabs(model: ChatModel, item: PhonePane, set: (PhonePane) -> Unit, modifier: Modifier = Modifier) {
    val p = Ink
    val tabs = item.memoryTabs()
    val on = item.arg.orEmpty()
    val nameOf = { id: String -> if (id.isEmpty()) "Main" else model.memories.firstOrNull { it.id == id }?.name ?: id }
    val shown = tabs.filter { it == on || it.isEmpty() || model.memories.isEmpty() || model.memories.any { m -> m.id == it } }
    val show = { id: String -> set(chatPane(id, if (id in tabs) tabs else tabs + id)) }
    var adding by remember { mutableStateOf(false) }
    Box(modifier) {
        Row(
            Modifier.height(44.dp).floating(p).padding(4.dp).horizontalScroll(rememberScrollState()),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            shown.forEach { id ->
                val active = id == on
                Row(
                    Modifier.height(36.dp).clip(CircleShape).then(if (active) Modifier.background(p.hover) else Modifier)
                        .clickable { show(id) }
                        .semantics { selected = active; contentDescription = "${nameOf(id)} tab" }
                        .padding(start = 14.dp, end = if (active && shown.size > 1) 4.dp else 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(nameOf(id), style = type(TypeScale.control, 1.3f, if (active) FontWeight.Medium else FontWeight.Normal), color = if (active) p.ink else p.mid, maxLines = 1)
                    if (active && shown.size > 1) Round(onClick = {
                        val keep = tabs.filter { it != id }
                        set(chatPane(keep[(tabs.indexOf(id) - 1).coerceAtLeast(0)], keep))
                    }, size = 28.dp) { Glyph(Glyph.close, 12.dp, p.soft, label = "Close ${nameOf(id)}") }
                }
            }
            Round(onClick = { adding = !adding }, size = 36.dp) { Glyph(Glyph.plus, 18.dp, p.mid, label = "Open a memory here") }
        }
        if (adding) Popup(
            alignment = Alignment.TopStart,
            offset = IntOffset(0, with(androidx.compose.ui.platform.LocalDensity.current) { 52.dp.roundToPx() }),
            onDismissRequest = { adding = false },
            properties = PopupProperties(focusable = true),
        ) {
            AddMemory(model, offered = (if ("" in tabs) emptyList() else listOf("" to "Main")) + model.memories.filter { it.id !in tabs }.map { it.id to it.name }) { id ->
                adding = false
                show(id)
            }
        }
    }
}

/** The +'s list: the memories not open, then a new one, a name and, if
 * wanted, its instructions, as Settings makes one. */
@Composable
private fun AddMemory(model: ChatModel, offered: List<Pair<String, String>>, pick: (String) -> Unit) {
    val p = Ink
    var making by remember { mutableStateOf(false) }
    var name by remember { mutableStateOf("") }
    var instructions by remember { mutableStateOf("") }
    var said by remember { mutableStateOf<String?>(null) }
    val row = Modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(Radii.lg)).padding(horizontal = 12.dp)
    Column(Modifier.width(280.dp).floating(p, RoundedCornerShape(Radii.xl)).padding(4.dp)) {
        offered.forEach { (id, label) ->
            Box(row.clickable { pick(id) }.semantics { contentDescription = "Open $label" }, contentAlignment = Alignment.CenterStart) {
                Text(label, style = Type.small, color = p.ink)
            }
        }
        if (!making) Box(row.clickable { making = true }, contentAlignment = Alignment.CenterStart) {
            Text("New memory…", style = Type.small, color = p.mid)
        } else Column(Modifier.padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val field = Modifier.fillMaxWidth().clip(RoundedCornerShape(Radii.lg)).background(p.paper).border(1.dp, p.frame, RoundedCornerShape(Radii.lg)).padding(horizontal = 12.dp, vertical = 10.dp)
            Box {
                BasicTextField(name, { name = it }, singleLine = true, textStyle = Type.small.copy(color = p.ink), cursorBrush = SolidColor(p.ink), modifier = field.semantics { contentDescription = "New memory's name" })
                if (name.isEmpty()) Text("Name, as Tutor", style = Type.small, color = p.soft, modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp))
            }
            Box {
                BasicTextField(instructions, { instructions = it }, textStyle = Type.small.copy(color = p.ink), cursorBrush = SolidColor(p.ink), modifier = field.heightIn(min = 72.dp).semantics { contentDescription = "New memory's instructions" })
                if (instructions.isEmpty()) Text("Instructions, if any", style = Type.small, color = p.soft, modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp))
            }
            said?.let { Text(it, style = Type.note, color = p.accent) }
            Box(
                Modifier.align(Alignment.End).height(36.dp).clip(CircleShape).background(p.hover)
                    .clickable(enabled = name.isNotBlank()) {
                        model.addMemory(name, instructions) { id, why -> if (id != null) pick(id) else said = why }
                    }.padding(horizontal = 16.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Make it", style = Type.small, color = if (name.isNotBlank()) p.ink else p.soft) }
        }
    }
}
