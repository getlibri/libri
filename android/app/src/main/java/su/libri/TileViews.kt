package su.libri

// The kinds of tile as the host declares them (tiles.rs, /api/tiles): their
// names, what their arg is, which open again and how, and the rows their
// menus add. The app keeps only what is its own: a place for each kind it
// draws (VIEWS; Places.kt draws them) and a glyph for each row (ROW_GLYPHS);
// nix/check_tile_coverage.py holds both to the host's list.

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import su.libri.core.PhonePane

class RowDef(val id: String, val label: String, val keys: String?)

class TileDef(
    val kind: String,
    val name: String,
    val phone: Boolean,
    val offered: Boolean,
    /** A second one's arg: a fresh name from this prefix, or this word. */
    val freshPrefix: String?,
    val freshWord: String?,
    val rows: List<RowDef>,
)

class TileRegistry(val tiles: List<TileDef>, val barRows: List<RowDef>) {
    fun of(kind: String) = tiles.firstOrNull { it.kind == kind }

    /** The kinds the plus offers that this app draws, in the host's order. */
    fun offered() = tiles.filter { it.offered && it.phone && it.kind in VIEWS }.map { it.kind }

    companion object {
        fun read(text: String): TileRegistry {
            val o = JSONObject(text)
            fun rows(a: org.json.JSONArray?) = (0 until (a?.length() ?: 0)).map { i ->
                val r = a!!.getJSONObject(i)
                RowDef(r.getString("id"), r.getString("label"), r.optString("keys").takeIf { r.has("keys") })
            }
            val all = o.getJSONArray("tiles")
            val tiles = (0 until all.length()).map { i ->
                val t = all.getJSONObject(i)
                val fresh = t.optJSONObject("fresh")
                TileDef(
                    kind = t.getString("kind"),
                    name = t.getString("name"),
                    phone = t.getJSONObject("runs").getBoolean("phone"),
                    offered = t.optBoolean("offered"),
                    freshPrefix = fresh?.takeIf { it.optString("is") == "name" }?.optString("prefix"),
                    freshWord = fresh?.takeIf { it.optString("is") == "word" }?.optString("word"),
                    rows = rows(t.optJSONArray("rows")),
                )
            }
            return TileRegistry(tiles, rows(o.optJSONArray("bar_rows")))
        }
    }
}

/** The kinds this app has a place for. */
val VIEWS = listOf("chat", "temp", "terminal", "thread", "library", "board", "browser", "system", "builtin", "pinned")

/** A glyph per row the host names. */
val ROW_GLYPHS: Map<String, ImageVector> = mapOf(
    "search" to Glyph.find,
    "copy-answer" to Glyph.files,
    "new-temp" to Glyph.chat,
    "promote" to Glyph.work,
    "paste" to Glyph.files,
    "clear" to Glyph.eraser,
    "restart" to Glyph.terminal,
    "stop" to Glyph.close,
    "archive" to Glyph.work,
    "reload" to Glyph.library,
    "window" to Glyph.library,
    "new-board" to Glyph.plus,
    "export" to Glyph.board,
    "move" to Glyph.panel,
    "edit-bar" to Glyph.panel,
    "close" to Glyph.close,
)

/** A row the app can run: what to do, and whether it can now; `then`, the
 * choices a second step offers (Move to workspace). */
class RowCode(val enabled: Boolean = true, val then: List<Pair<String, () -> Unit>>? = null, val run: () -> Unit = {})

/** `pane` as a new item beside `items`: a kind the host says gives its
 * second one an arg of its own gets it (a terminal a shell of its own, a
 * Temp or a whiteboard a new one); any other is the same again. */
fun another(registry: TileRegistry?, pane: PhonePane, items: List<PhonePane>): PhonePane {
    val t = registry?.of(pane.kind) ?: return pane
    if (pane.arg != null || items.none { it.kind == pane.kind && it.arg == null }) return pane
    t.freshPrefix?.let { return PhonePane(pane.kind, it + freshSpaceId().drop(1)) }
    t.freshWord?.let { return PhonePane(pane.kind, it) }
    return pane
}

/** An item's menu, as the web's (tileMenu.tsx): the rows the host names for
 * its kind with code here, then the bar's, the close apart; drawn as the
 * bar's other menus, over the bar. */
@Composable
fun ItemMenu(title: String, own: List<RowDef>, bar: List<RowDef>, code: Map<String, RowCode?>, close: () -> Unit) {
    val p = Ink
    var step by remember { mutableStateOf<Pair<String, List<Pair<String, () -> Unit>>>?>(null) }
    androidx.activity.compose.BackHandler { if (step != null) step = null else close() }
    Box(Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { close() } }) {
        Column(
            Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = FOOT_GAP + BAR_HEIGHT + FOOT_GAP)
                .width(280.dp)
                .shadow(8.dp, RoundedCornerShape(Radii.xl), ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
                .clip(RoundedCornerShape(Radii.xl)).background(p.lift).border(1.dp, p.hair, RoundedCornerShape(Radii.xl))
                .pointerInput(Unit) { detectTapGestures { } }
                .verticalScroll(rememberScrollState()).padding(4.dp)
                .semantics { contentDescription = "$title menu" },
        ) {
            val row = @Composable { label: String, glyph: ImageVector?, keys: String?, enabled: Boolean, go: () -> Unit ->
                Row(
                    Modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(Radii.lg)).clickable(enabled = enabled, onClick = go).padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) { glyph?.let { Glyph(it, 18.dp, if (enabled) p.mid else p.soft) } }
                    Text(label, style = Type.small, color = if (enabled) p.ink else p.soft, modifier = Modifier.weight(1f), maxLines = 1)
                    keys?.let { Text(it, style = Type.note, color = p.soft) }
                }
            }
            val s = step
            if (s != null) {
                Text(s.first, style = Type.note, color = p.soft, modifier = Modifier.padding(start = 12.dp, top = 8.dp, bottom = 4.dp))
                s.second.forEach { (label, go) -> row(label, null, null, true) { close(); go() } }
                return@Column
            }
            val shown = { defs: List<RowDef> ->
                defs.mapNotNull { d -> code[d.id]?.let { d to it } }
            }
            val groups = listOf(shown(own), shown(bar.filter { it.id != "close" }), shown(bar.filter { it.id == "close" })).filter { it.isNotEmpty() }
            groups.forEachIndexed { i, g ->
                if (i > 0) Box(Modifier.padding(vertical = 4.dp, horizontal = 12.dp).fillMaxWidth().height(1.dp).background(p.hair))
                g.forEach { (d, c) ->
                    row(d.label, ROW_GLYPHS[d.id], d.keys, c.enabled) {
                        val then = c.then
                        if (then != null) step = d.label.removeSuffix("…") to then
                        else {
                            close()
                            c.run()
                        }
                    }
                }
            }
        }
    }
}
