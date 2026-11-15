package su.libri

import android.content.Context
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import java.text.DateFormat
import java.util.Date
import org.json.JSONArray
import org.json.JSONObject
import su.libri.core.Message
import su.libri.core.Role

/** A message kept to find again (pins.tsx): its words, so it outlives the
 * history the app has loaded. */
data class Pin(val id: String, val mine: Boolean, val text: String, val date: Long?, val at: Long)

/** Pins and whether the Pinned place shows, kept per device, as the page
 * keeps them. */
class Pins(context: Context) {
    private val prefs = context.getSharedPreferences("libri-pins", Context.MODE_PRIVATE)
    val list = mutableStateListOf<Pin>().apply { addAll(load()) }
    val on = mutableStateOf(prefs.getBoolean("on", true))

    private fun load(): List<Pin> = runCatching {
        val a = JSONArray(prefs.getString("pins", "[]"))
        (0 until a.length()).map { i ->
            val o = a.getJSONObject(i)
            Pin(o.getString("id"), o.getBoolean("mine"), o.getString("text"), o.optLong("date").takeIf { it > 0 }, o.getLong("at"))
        }
    }.getOrDefault(emptyList())

    private fun save() {
        val a = JSONArray()
        list.forEach { p ->
            a.put(JSONObject().put("id", p.id).put("mine", p.mine).put("text", p.text).put("date", p.date ?: 0).put("at", p.at))
        }
        prefs.edit().putString("pins", a.toString()).apply()
    }

    /** Matched by id, or by its words once a reload has renumbered it. */
    fun has(m: Message) = list.any { it.id == m.id || it.text == m.words() }

    fun toggle(m: Message) {
        if (has(m)) list.removeAll { it.id == m.id || it.text == m.words() }
        else list.add(0, Pin(m.id, m.role == Role.USER, m.words(), m.date, System.currentTimeMillis()))
        save()
    }

    fun drop(p: Pin) {
        list.remove(p)
        save()
    }

    fun turn(on: Boolean) {
        this.on.value = on
        prefs.edit().putBoolean("on", on).apply()
    }
}

/** What bringing a pin back sends: the pinned words, for the chat to pick up. */
fun resumed(p: Pin, name: String): String {
    val who = if (p.mine) "you" else name
    val whenSaid = p.date?.let { DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(it)) } ?: "earlier"
    return "Picking up from a pinned message ($who, $whenSaid):\n\n" + p.text.lines().joinToString("\n") { "> $it" }
}

/** The words that ask the chat to promote (work.tsx `promotion`). */
fun promotion(directions: String, id: String?, text: String?): String {
    val said = directions.trim().ifEmpty { "carry out what we just settled" }
    if (text == null) return "Create task: $said"
    val hippo = id?.let { Regex("^h-(\\d+)$").find(it)?.groupValues?.get(1) }
    val quote = text.trim().lines().take(4).joinToString("\n").take(400)
    return "Create task: $said\n\n${if (hippo != null) "From message $hippo:" else "From this:"}\n> ${quote.replace("\n", "\n> ")}"
}
