package su.libri

import android.content.Context
import android.net.Uri
import androidx.compose.ui.text.TextRange
import java.io.File
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject
import su.libri.core.Attachment

/** A draft as it goes to disk: one value, so a change of any part is seen. */
data class KeptDraft(
    val text: String,
    val start: Int,
    val end: Int,
    val files: List<Picked>,
) {
    val empty get() = text.isEmpty() && files.isEmpty()
}

fun Draft.kept() = KeptDraft(text, typed.selection.start, typed.selection.end, files.toList())

/**
 * What is being written, on disk, one file to a draft: the activity and the
 * process can both go while the person is away. A chosen file is copied in,
 * since the grant on its content URI ends with the process.
 */
class Drafts(private val context: Context) {
    private val root = context.filesDir.resolve("drafts")
    private val files = root.resolve("files")

    init {
        files.mkdirs()
        sweep()
    }

    /** The draft saved as `key`; an empty one if there is none. */
    fun read(key: String): Draft {
        val d = Draft()
        val saved = runCatching { JSONObject(path(key).readText()) }.getOrNull() ?: return d
        val text = saved.optString("text")
        val most = text.length
        d.typed.edit {
            replace(0, length, text)
            selection = TextRange(saved.optInt("start", most).coerceIn(0, most), saved.optInt("end", most).coerceIn(0, most))
        }
        val list = saved.optJSONArray("files") ?: JSONArray()
        for (i in 0 until list.length()) {
            val f = list.getJSONObject(i)
            d.files += Picked(Uri.parse(f.getString("uri")), f.getString("name"), f.getString("mime"))
        }
        return d
    }

    /** Saves a draft in full; an empty one leaves nothing on disk. */
    @Synchronized
    fun write(key: String, d: KeptDraft) {
        val at = path(key)
        if (d.empty) {
            at.delete()
            return
        }
        val list = JSONArray()
        d.files.forEach {
            list.put(JSONObject().put("uri", held(it).toString()).put("name", it.name).put("mime", it.mime))
        }
        val saved = JSONObject().put("text", d.text).put("start", d.start).put("end", d.end).put("files", list)
        val tmp = File("${at.path}.tmp")
        tmp.writeText(saved.toString())
        tmp.renameTo(at)
    }

    /** A file taken back from the core, held here for the box. */
    fun hold(a: Attachment): Picked {
        val to = files.resolve(UUID.randomUUID().toString())
        to.writeBytes(a.data)
        return Picked(Uri.fromFile(to), a.name, a.mime)
    }

    /** The copy of a chosen file, made the first time it is saved. One that
     * can't be read stays as it was picked: it may still read later. */
    private fun held(p: Picked): Uri {
        if (p.uri.scheme == "file") return p.uri
        val to = files.resolve(UUID.nameUUIDFromBytes(p.uri.toString().toByteArray()).toString())
        if (to.exists()) return Uri.fromFile(to)
        val tmp = File("${to.path}.tmp")
        val copied = runCatching {
            context.contentResolver.openInputStream(p.uri)!!.use { from -> tmp.outputStream().use { from.copyTo(it) } }
            tmp.renameTo(to)
        }.getOrDefault(false)
        return if (copied) Uri.fromFile(to) else p.uri
    }

    /** Clears files no saved draft names. Only here, as the app starts: a
     * message sent from the box is still reading its file for a while after
     * the box lets it go. */
    private fun sweep() {
        val named = root.listFiles { f -> f.isFile }.orEmpty().flatMap { saved ->
            val list = runCatching { JSONObject(saved.readText()).getJSONArray("files") }.getOrNull() ?: JSONArray()
            (0 until list.length()).map { list.getJSONObject(it).getString("uri") }
        }.toSet()
        files.listFiles().orEmpty().filter { Uri.fromFile(it).toString() !in named }.forEach { it.delete() }
    }

    private fun path(key: String) = root.resolve("$key.json")
}
