package su.libri

import android.app.Application
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.core.content.edit
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.time.OffsetDateTime
import su.libri.core.WorkChoice
import su.libri.core.NamedMemory
import su.libri.core.Talk
import su.libri.core.Attachment
import su.libri.core.Chat
import su.libri.core.memoryAsked
import su.libri.core.ToolCall
import su.libri.core.ChatListener
import su.libri.core.ClientException
import su.libri.core.Host
import su.libri.core.HostInfo
import su.libri.core.PhoneLayout
import su.libri.core.PhonePane
import su.libri.core.PhoneSpace
import su.libri.core.PhoneClosed
import su.libri.core.Account
import su.libri.core.Limit
import su.libri.core.Usage
import su.libri.core.Link
import su.libri.core.Me
import su.libri.core.MemoryHealth
import su.libri.core.Models
import su.libri.core.Session
import su.libri.core.Message
import su.libri.core.Quote
import su.libri.core.Ask
import su.libri.core.StepKind
import su.libri.core.Thread
import su.libri.core.address as wholeAddress
import su.libri.core.probe

/** A file chosen in the composer, not yet sent. */
data class Picked(val uri: Uri, val name: String, val mime: String) {
    val image get() = mime.startsWith("image/")
}

/**
 * The host this phone talks to and its conversation. The core follows the
 * host on a thread of its own while the app is in view (start/stop follow
 * the activity), and reports each change here.
 */
class ChatModel(app: Application) : AndroidViewModel(app) {
    private val prefs = app.getSharedPreferences("libri", Context.MODE_PRIVATE)

    /** The host's address, once one has answered; null until then. */
    var address by mutableStateOf(prefs.getString(ADDRESS, null))
        private set
    var info by mutableStateOf<HostInfo?>(null)
        private set
    var connecting by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
        private set

    /** Every host this phone knows (hosts.ts), the one in use among them.
     * An install from before the list starts it with its one saved host. */
    var hosts by mutableStateOf(savedHosts())
        private set

    /** Each known host's latency in ms, or null for down, by address. */
    var hostsSeen by mutableStateOf<Map<String, Long?>>(emptyMap())
        private set

    /** The host entry is open over the host in use, which it leaves alone
     * until another answers. */
    var adding by mutableStateOf(false)
        private set

    /** The host's requests, for whatever screen needs them. */
    var host by mutableStateOf(address?.let { runCatching { Host(it) }.getOrNull() })
        private set
    var models by mutableStateOf<Models?>(null)
        private set
    /** Every subscription's windows, kept so the rings draw on start. */
    var usage by mutableStateOf(savedUsage())
        private set
    var me by mutableStateOf<Me?>(null)
        private set

    /** The product's name, which a composer asks to be messaged by: Libri,
     * whatever the assistant on this host calls itself (`/api/me` app). */
    val app get() = me?.app?.ifBlank { null } ?: "Libri"

    /** What a pick or send last failed with, shown once. */
    var notice by mutableStateOf<String?>(null)

    /** The one reading of the memory, so the line, the panel's row and the
     * Memory page can't disagree. Null is nothing to show: not asked yet, or
     * an older host without the route. */
    var memory by mutableStateOf<MemoryHealth?>(null)
        private set

    /** The line or a row asked for the Memory page; the panel takes it up. */
    var toMemory by mutableStateOf(false)
    /** Opens Settings at Connections, as a built-in's empty view asks. */
    var toConnections by mutableStateOf(false)

    // Host up but no status: when it began. Held over a second check, so one
    // slow answer on a loaded host never raises the line.
    private var silent: String? = null

    /** Asks how the memory is (memory.tsx useMemory). The host's own clock for
     * a silent memory counts from these reads. */
    suspend fun checkMemory() {
        val h = host ?: return
        val read = withContext(Dispatchers.IO) { runCatching { h.memory() } }
        val up = read.isFailure && read.exceptionOrNull() !is ClientException.SignedOut &&
            withContext(Dispatchers.IO) { runCatching { h.info() } }.isSuccess
        if (host !== h) return
        when {
            read.isSuccess -> { silent = null; memory = read.getOrNull() }
            !up -> { silent = null; memory = MemoryHealth("unknown", null, null, null, null, null, null, null) }
            silent == null -> silent = OffsetDateTime.now().toString()
            else -> memory = MemoryHealth("failing", "The memory status isn't answering.", silent, null, null, null, null, null)
        }
    }

    /** Light, dark or as the system is; the talk's text size. */
    var theme by mutableStateOf(prefs.getString("theme", "system") ?: "system")
        private set
    /** The talk's text size in the page's pixels, as the web's pinch keeps it. */
    var chatDetail by mutableStateOf(prefs.getString("chat-detail", "standard") ?: "standard")
        private set

    private var threadPresentation by mutableStateOf<Map<String, List<Message>>>(emptyMap())

    fun threadMessages(name: String): List<Message> = threadPresentation[name].orEmpty()


    fun pickChatDetail(value: String) {
        require(value in listOf("simple", "standard", "full"))
        chatDetail = value
        prefs.edit { putString("chat-detail", value) }
        follow.reset()
        temps.values.forEach { it.view.follow.reset() }
        threadViews.values.forEach { it.follow.reset() }
    }

    var textPx by mutableStateOf(prefs.getInt("text-px", 16))
        private set

    fun pickTheme(t: String) {
        theme = t
        prefs.edit { putString("theme", t) }
    }

    /** Whether dictation runs on this phone's own recognizer rather than
     * going to the host. */
    var dictateOnPhone by mutableStateOf(prefs.getBoolean("dictate-on-phone", false))
        private set

    fun pickDictateOnPhone(on: Boolean) {
        dictateOnPhone = on
        prefs.edit { putBoolean("dictate-on-phone", on) }
    }

    fun pickTextPx(px: Int) {
        textPx = px.coerceIn(TEXT_MIN, TEXT_MAX)
        prefs.edit { putInt("text-px", textPx) }
    }

    /** The Work page's threads, asked again while it is in view. */
    var threads by mutableStateOf<List<Thread>>(emptyList())
        private set

    /** The messages between agents in the last few seconds, for the tree's
     * dashes (task::Talk). */
    var talk by mutableStateOf<List<Talk>>(emptyList())
        private set

    /** The named memories, the tree's roots beside Main, each with whether
     * its chat works or waits on the person. */
    var memories by mutableStateOf<List<NamedMemory>>(emptyList())
        private set

    fun refreshWork() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { h.work() }.onSuccess { if (host === h) { threads = it.threads; talk = it.talk } }
            runCatching { h.memories() }.onSuccess { if (host === h) memories = it }
        }
    }

    /** Makes a named memory (a chat tab's "New memory…"); `done` gets its id,
     * or why not. */
    fun addMemory(name: String, instructions: String, done: (id: String?, why: String?) -> Unit) {
        val h = host ?: return done(null, "Not connected.")
        viewModelScope.launch(Dispatchers.IO) {
            val made = runCatching { h.addMemory(name.trim(), instructions) }
            runCatching { h.memories() }.onSuccess { if (host === h) withContext(Dispatchers.Main) { memories = it } }
            withContext(Dispatchers.Main) {
                made.fold({ done(it, null) }, { e -> done(null, (e as? ClientException.Said)?.message ?: "That didn't reach the host.") })
            }
        }
    }

    /** What came of the last worker started from Tasks: its name, or why not. */
    var started by mutableStateOf<String?>(null)

    /** Starts a worker on the person's own words, not through the main chat. */
    /** Starts a worker on the sheet's words and files, on `choice` as its
     * pickers left it. A failure puts them back in the sheet's draft. */
    fun startWorker(brief: String, files: List<Picked>, choice: WorkChoice?) {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val said = runCatching { h.workStart(brief, files.mapNotNull { attachment(it) }.map { h.upload(it) }, choice) }
                .fold({ "Started $it. Its report comes to its card here." }, { e ->
                    withContext(Dispatchers.Main) {
                        if (brief.isNotEmpty()) taskDraft.text = joined(taskDraft.text, brief)
                        taskDraft.files += files.filterNot { it in taskDraft.files }
                    }
                    (e as? ClientException.Said)?.message ?: "The worker did not start."
                })
            withContext(Dispatchers.Main) {
                started = said
                refreshWork()
            }
            delay(8_000)
            withContext(Dispatchers.Main) { if (started == said) started = null }
        }
    }

    /** The Library: pages show_page published and the book's lists. */
    var library by mutableStateOf<su.libri.core.LibraryShelf?>(null)
        private set

    /** A Library page asked open, by a link tapped or by show_page publishing
     * it live; the Library takes it and clears it. */
    var opening by mutableStateOf<String?>(null)
    private var published: su.libri.core.Published? = null

    /** The page a link names, if it is a Libri page link (`#page=<name>`,
     * library.rs link) at this host or at app.libri.is, the address the
     * links are written with (pageLink.ts pageOf). */
    fun pageOf(link: String): String? {
        val u = runCatching { android.net.Uri.parse(link) }.getOrNull() ?: return null
        val here = address?.let { runCatching { android.net.Uri.parse(it) }.getOrNull() }
        val ours = u.host == "app.libri.is" || (here != null && u.host == here.host && u.port == here.port)
        if (!ours) return null
        val fragment = u.fragment ?: return null
        return fragment.split('&').firstNotNullOfOrNull { kv ->
            kv.split('=', limit = 2).takeIf { it.size == 2 && it[0] == "page" }?.get(1)
        }?.takeIf { it.isNotEmpty() }
    }

    fun refreshLibrary() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.library() }.onSuccess { if (host === h) library = it } }
    }

    /** The skills each harness finds, for the composer's `$` menu; asked
     * each time a `$` opens it, so a skill just made shows. */
    var skillMenus by mutableStateOf<List<su.libri.core.SkillMenu>>(emptyList())
        private set

    fun refreshSkills() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.skillMenus() }.onSuccess { if (host === h) skillMenus = it } }
    }

    /** The main chat's harness and its own slash commands, for the `/`
     * menu beside Libri's (chat.ts `harnessCommands`); asked as it opens. */
    var harnessCommands by mutableStateOf<su.libri.core.HarnessCommands?>(null)
        private set

    fun refreshCommands() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.harnessCommands() }.onSuccess { if (host === h) harnessCommands = it } }
    }

    /** The tools whose calls draw a card (MCP Apps), by the names calls
     * carry; asked once per host, as the web page asks (apps.ts). */
    var cardTools by mutableStateOf<Set<String>>(emptySet())
        private set
    /** Of those, the tools whose card opens by itself (apps.rs `Card::opens`). */
    var cardOpens by mutableStateOf<Set<String>>(emptySet())
        private set

    fun refreshCards() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.cardTools() }.onSuccess { if (host === h) { cardTools = it.map { c -> c.name }.toSet(); cardOpens = it.filter { c -> c.opens }.map { c -> c.name }.toSet() } } }
    }


    /** The host's page of one call's card, in the chat or made large. */
    fun callCardUrl(drawn: su.libri.core.DrawnCard, t: ToolCall, fresh: Boolean, full: Boolean): String? =
        host?.callCardUrl(drawn.name, drawn.input, t.structured.takeUnless { drawn.shown }, t.result, t.error, fresh, full)

    /** A call's card made large over everything, as the host's page of it. */
    var fullCard by mutableStateOf<String?>(null)

    /** Where a published page opens, on the host in use. */
    fun pageUrl(name: String): String? = host?.pageUrl(name)

    /** The built-ins (Mail, Calendar) as the host has them. */
    var builtins by mutableStateOf<List<su.libri.core.Builtin>>(emptyList())

    fun refreshBuiltins() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.builtins() }.onSuccess { if (host === h) builtins = it } }
    }

    // A phone's workspaces (phone.tsx, layout.rs Store::Phone): each a row
    // of items on the bar at the foot, kept on the host apart from the wide
    // page's tiles, so the web on a phone and this app show the same. The
    // copy here draws at once and is kept between launches; nothing goes to
    // the host before it has answered, so a stale copy never overwrites its.
    var spaces by mutableStateOf(cachedSpaces()?.first ?: firstSpaces().first)
        private set
    var spaceOn by mutableStateOf(cachedSpaces()?.second ?: spaces.first().id)
        private set
    /** The agent's latest change to them, while it may be undone in one tap. */
    var spacesByAgent by mutableStateOf(false)
        private set
    var spacesSaveError by mutableStateOf(false)
        private set
    private var spacesVersion: ULong? = null
    /** Recently closed, newest first: kept on the host 30 days. */
    var spacesClosed by mutableStateOf<List<PhoneClosed>>(emptyList())
        private set
    private var spacesSent: Triple<List<PhoneSpace>, String, List<PhoneClosed>>? = null
    private var spacesSave: kotlinx.coroutines.Job? = null

    private fun cachedSpaces(): Pair<List<PhoneSpace>, String>? = runCatching {
        val o = JSONObject(prefs.getString("phone-spaces:$address", null) ?: return null)
        val list = o.getJSONArray("spaces").let { a ->
            (0 until a.length()).map { i ->
                val w = a.getJSONObject(i)
                val items = w.getJSONArray("items").let { it2 ->
                    (0 until it2.length()).map { j ->
                        val p = it2.getJSONObject(j)
                        PhonePane(p.getString("kind"), p.optString("arg").takeIf { p.has("arg") })
                    }
                }
                PhoneSpace(w.getString("id"), w.getString("name"), items)
            }
        }.let(::withoutWorkers)
        if (list.isEmpty() || list.any { it.items.isEmpty() }) null
        else list to o.getString("on").let { on -> if (list.any { it.id == on }) on else list.first().id }
    }.getOrNull()

    private fun cacheSpaces() {
        val a = JSONArray()
        spaces.forEach { w ->
            val items = JSONArray()
            w.items.forEach { p -> items.put(JSONObject().put("kind", p.kind).apply { p.arg?.let { put("arg", it) } }) }
            a.put(JSONObject().put("id", w.id).put("name", w.name).put("items", items))
        }
        prefs.edit { putString("phone-spaces:$address", JSONObject().put("spaces", a).put("on", spaceOn).toString()) }
    }

    /** Keeps the phone's workspaces as the person left them; the host has
     * them once it has answered, where a drag's many changes settle. */
    fun changeSpaces(to: List<PhoneSpace>, on: String, closed: List<PhoneClosed> = spacesClosed) {
        spaces = to
        spaceOn = on
        spacesClosed = closed
        spacesByAgent = false
        cacheSpaces()
        spacesSave?.cancel()
        val h = host ?: return
        if (spacesVersion == null || spacesSent == Triple(to, on, closed)) return
        spacesSave = viewModelScope.launch(Dispatchers.IO) {
            delay(600)
            while (true) {
                val kept = runCatching { h.keepPhoneLayout(to, on, closed) }
                if (host !== h) return@launch
                val got = kept.getOrNull()
                withContext(Dispatchers.Main) {
                    spacesSaveError = got == null
                    if (got != null) {
                        spacesVersion = got.version
                        spacesSent = Triple(to, on, closed)
                    }
                }
                if (got != null) return@launch
                delay(5_000)
            }
        }
    }

    /** Follows the host's copy while the app is in view: each answer waits
     * for the next version, so a change from the agent or the web shows at once. */
    suspend fun followSpaces() {
        val h = host ?: return
        // Another host's versions mean nothing here.
        spacesVersion = null
        spacesSent = null
        while (host === h) {
            val got = runCatching { withContext(Dispatchers.IO) { h.phoneLayout(spacesVersion) } }.getOrNull()
            if (host !== h) return
            if (got == null) {
                delay(5_000)
                continue
            }
            if (got.version != spacesVersion) {
                if (spacesSave?.isActive == true) spacesVersion = got.version else adoptSpaces(got)
            }
        }
    }

    private fun adoptSpaces(k: PhoneLayout) {
        spacesVersion = k.version
        // Nothing kept yet: the first run's stands, sent on its first change.
        if (k.spaces.isEmpty() || k.spaces.any { it.items.isEmpty() }) return
        spaces = withoutWorkers(k.spaces)
        spaceOn = if (k.spaces.any { it.id == k.on }) k.on else k.spaces.first().id
        spacesClosed = k.closed
        spacesSent = Triple(spaces, spaceOn, spacesClosed)
        spacesByAgent = k.by == "agent" && k.undo
        cacheSpaces()
    }

    /** Closes a workspace into Recently closed; the last one closed leaves
     * a fresh one. Answers where it stood, for its Undo. */
    fun closeSpace(id: String): Int {
        val i = spaces.indexOfFirst { it.id == id }
        if (i < 0) return -1
        val w = spaces[i]
        val gone = PhoneClosed(w.id, w.name, w.items, (System.currentTimeMillis() / 1000).toULong())
        val closed = listOf(gone) + spacesClosed.filter { it.id != id }
        val rest = spaces.filter { it.id != id }
        if (rest.isEmpty()) firstSpaces().let { (s, on) -> changeSpaces(s, on, closed) }
        else changeSpaces(rest, if (spaceOn == id) rest[(i - 1).coerceAtLeast(0)].id else spaceOn, closed)
        return i
    }

    /** Back from Recently closed, at `at` (else the end), shown. */
    fun reopenSpace(id: String, at: Int? = null) {
        val c = spacesClosed.firstOrNull { it.id == id } ?: return
        val back = PhoneSpace(c.id, c.name, c.items)
        val list = spaces.toMutableList().apply { add((at ?: size).coerceIn(0, size), back) }
        changeSpaces(list, id, spacesClosed.filter { it.id != id })
    }

    /** Deletes a closed workspace for good. */
    fun forgetSpace(id: String) = changeSpaces(spaces, spaceOn, spacesClosed.filter { it.id != id })

    fun undoSpaces() {
        val h = host ?: return
        spacesByAgent = false
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { h.undoPhoneLayout() }.onSuccess { k -> withContext(Dispatchers.Main) { adoptSpaces(k) } }
        }
    }

    fun keepAgentSpaces() {
        spacesByAgent = false
    }

    /** Where a built-in's view opens alone, on the host in use. */
    fun builtinUrl(id: String): String? = host?.builtinUrl(id)

    /** The Whiteboard tile alone, on `id` or the host's first board. */
    fun boardUrl(id: String?): String? = host?.boardUrl(id)

    /** The Browser tile's page: Libri's browser, shared with the agents. */
    fun browserUrl(): String? = host?.browserTileUrl()

    /** The System tile's page: the host's CPU, memory, disks and workers. */
    fun systemUrl(): String? = host?.systemTileUrl()

    /** Whether this device may only read (the host's pairing scope). */
    val readOnly: Boolean get() = me?.scope == "read"

    /** A published page's still at a version, for its Library card. */
    fun snapshotUrl(name: String, changed: ULong): String? = host?.snapshotUrl(name, changed)

    /** The watches and reminders, listed under the workers on Tasks. */
    var watches by mutableStateOf<List<su.libri.core.WatchItem>>(emptyList())
        private set

    fun refreshWatches() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.watches() }.onSuccess { if (host === h) watches = it } }
    }

    /** Snooze and delete edit the book directly, with no model call. */
    fun watchDelete(id: ULong) {
        watches = watches.filter { it.id != id }
        act(then = ::refreshWatches) { host?.watchDelete(id) }
    }

    fun watchSnooze(id: ULong, until: Long) {
        watches = watches.map { if (it.id == id) it.copy(at = until.toULong()) else it }
        act(then = ::refreshWatches) { host?.watchSnooze(id, until.toULong()) }
    }

    fun workAct(name: String, act: String) = act(then = ::refreshWork) { host?.workAct(name, act) }

    /** The host's word on the last merge, for the thread it was of: shown
     * under that thread's header, as the web shows it. */
    var landed by mutableStateOf<Pair<String, String>?>(null)

    /** Merges a thread into the main chat's memory, as though done there, and
     * archives it. */
    fun workMerge(id: String, name: String) {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val said = try {
                h.workMerge(id).ifBlank { "Merged." }
            } catch (e: ClientException) {
                e.message ?: "$name wasn't merged."
            }
            withContext(Dispatchers.Main) {
                landed = id to said
                refreshWork()
            }
        }
    }

    /** Puts a thread down for a nap until `until`, in seconds since the epoch. */
    fun workNap(name: String, until: Long) = act(then = ::refreshWork) { host?.workNap(name, until.toULong()) }

    /** One Temp: a full chat the host runs and keeps, which stays out of
     * the memory and goes on with its tab closed. `key` is the Temp's id as
     * its tab holds it; "" the host's current Temp, as before there were
     * several (scratchpad.rs). */
    // Stable: its state is snapshot state; said here, Compose needs no
    // generated stability field, which an inner class's lacks under R8.
    @androidx.compose.runtime.Stable
    inner class TempChat(val key: String) {
        private val id get() = key.ifEmpty { null }
        var tempId by mutableStateOf<String?>(null)
            private set
        var busy by mutableStateOf(false)
            internal set
        var unread by mutableStateOf(false)
        var stopping by mutableStateOf(false)
            internal set
        var asks by mutableStateOf<List<Ask>>(emptyList())
            internal set
        var open = false
        private var followed = false
        /** Its view, with its draft, kept apart from any other Temp's. */
        val view = ThreadView(kept(if (key.isEmpty()) "temp" else "temp:$key"), plain = true)

        /** Counts what the person did to it. A read begun before the latest
         * of it is stale: it must not bring back a Temp they closed, flip
         * Memory back, or end a turn they just started. */
        internal var acts = 0

        fun messages(): List<Message> = tempId?.let(::threadMessages).orEmpty()

        suspend fun read() {
            val h = host ?: return
            view.reading.withLock {
                val was = acts
                runCatching { withContext(Dispatchers.IO) { h.scratchpad(id) } }.onSuccess { t ->
                    if (host !== h || was != acts) return@onSuccess
                    if (busy && !t.working && !open) unread = true
                    tempId = t.id
                    busy = t.working
                    stopping = t.stopping
                    tempMemory = t.memory
                    asks = t.asks
                    view.heard(t.steps)
                }
            }
        }

        /** Reads on through a turn, for the bar's mark with the tab closed. */
        fun follow() {
            if (followed) return
            followed = true
            viewModelScope.launch {
                try {
                    while (busy) {
                        delay(TEMP_READ_MS)
                        read()
                    }
                } finally {
                    followed = false
                }
            }
        }

        /** As `tell`: the words show at once, and go back in the box if refused. */
        fun tell(text: String, files: List<Picked>) {
            // With no host the words stay in the box, and nothing waits on a turn.
            val h = host ?: return
            val telling = text.takeIf { it.isNotEmpty() }?.let(view::tell)
            acts++
            busy = true
            viewModelScope.launch {
                try {
                    withContext(Dispatchers.IO) {
                        h.scratchpadTell(text, files.mapNotNull { attachment(it) }.map { h.upload(it) }, id)
                    }
                    read()
                    follow()
                } catch (e: ClientException) {
                    busy = false
                    telling?.let { view.telling.remove(it) }
                    if (text.isNotEmpty()) view.draft.text = joined(view.draft.text, text)
                    view.draft.files += files.filterNot { it in view.draft.files }
                    notice = e.message
                }
            }
        }

        fun stop() {
            acts++
            stopping = true
            act(failed = { stopping = false }) { host?.scratchpadStop(id) }
        }

        /** What a host switch or a clear leaves of it: nothing. */
        internal fun forget() {
            acts++
            view.steps = emptyList()
            view.telling.clear()
            view.follow.reset()
            busy = false
            stopping = false
            unread = false
            asks = emptyList()
        }

        /** A fresh Temp: the host removes this one and any turn on it. The
         * Memory switch is the household's, and stays. */
        fun clear() {
            forget()
            act(failed = { viewModelScope.launch { read() } }) { host?.scratchpadClose(id) }
        }

        /** Into the chat as the talk it was, starting no turn; or its words
         * to a Work thread of its own, as a brief. */
        fun promote(work: Boolean) {
            if (!work) {
                forget()
                act(failed = { viewModelScope.launch { read() } }) { host?.scratchpadPromote(id) }
                return
            }
            val name = me?.name ?: "Assistant"
            val talk = (view.steps.orEmpty().mapNotNull {
                when (it.kind) {
                    StepKind.TOLD, StepKind.TASK -> "Me: ${it.text}"
                    StepKind.WORDS -> "$name: ${it.text}"
                    else -> null
                }
            } + view.telling.map { "Me: ${it.text}" }).joinToString("\n\n")
            send("${promotion("work on what this temp chat was about", null, null)}\n\n(The temp chat)\n$talk", emptyList())
            clear()
        }
    }

    /** Counts Reloads of the Library's page from its item's menu. */
    var libraryDrawn by mutableIntStateOf(0)

    /** Each open terminal's actions, for its item's menu (Terminal.kt). */
    val terminalActions = mutableMapOf<String, TerminalActions>()

    /** The kinds of tile as the host declares them (/api/tiles, TileViews.kt). */
    var tiles by mutableStateOf<TileRegistry?>(null)
        private set

    fun loadTiles() {
        val h = host ?: return
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) { h.tiles() } }.onSuccess { tiles = TileRegistry.read(it) }
        }
    }

    /** Every Temp a tab holds, by key; "" the host's current one. */
    val temps = mutableStateMapOf<String, TempChat>()
    fun temp(key: String = ""): TempChat = temps.getOrPut(key) { TempChat(key) }

    /** The Memory switch, which every Temp shares (scratchpad.rs `MEMORY`). */
    var tempMemory by mutableStateOf(true)
        private set

    /** Reads every Temp a tab holds, as a word from the host that one
     * changed asks. */
    fun readTemps() {
        if (temps.isEmpty()) temp()
        temps.values.toList().forEach { t ->
            viewModelScope.launch {
                t.read()
                t.follow()
            }
        }
    }

    /** A new Temp on the host, for a tab opened beside one: its id. */
    suspend fun newTemp(): String? = host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.scratchpadNew() } }.getOrNull() }

    /** The named memory's chat a chat tab shows (MemoryTabs.kt), read as
     * Temp's is. Each memory keeps its own talk and draft. */
    var memoryOpen by mutableStateOf<String?>(null)
        private set
    /** A memory asked open from the Workers tree or `/memory <name>`: its
     * tab in the chat opens or comes forward ("" for Main). */
    var memoryTabAsked by mutableStateOf<String?>(null)
    var memoryBusy by mutableStateOf(false)
        private set
    var memoryStopping by mutableStateOf(false)
        private set
    var memoryAsks by mutableStateOf<List<Ask>>(emptyList())
        private set
    private var memoryThread by mutableStateOf<String?>(null)
    private val memoryViews = mutableMapOf<String, ThreadView>()

    fun memoryView(id: String) = memoryViews.getOrPut(id) { ThreadView(kept("memory/$id"), plain = true) }

    /** What the open memory's chat is doing now, off the stream. */
    fun memoryMessages(): List<Message> = memoryThread?.let(::threadMessages).orEmpty()

    /** Shows a memory's chat; the same one again reads it afresh. */
    fun openMemory(id: String) {
        if (memoryOpen != id) {
            memoryThread = null
            memoryBusy = false
            memoryStopping = false
            memoryAsks = emptyList()
        }
        memoryOpen = id
    }

    suspend fun readMemoryChat(id: String) {
        val h = host ?: return
        val view = memoryView(id)
        view.reading.withLock {
            runCatching { withContext(Dispatchers.IO) { h.memoryChat(id) } }.onSuccess { t ->
                if (host !== h || memoryOpen != id) return@onSuccess
                memoryThread = t.id
                memoryBusy = t.working
                memoryStopping = t.stopping
                memoryAsks = t.asks
                view.heard(t.steps)
            }
        }
    }

    /** As Temp's: the words show at once, and go back in the box if refused. */
    fun tellMemory(id: String, text: String, files: List<Picked>) {
        val h = host ?: return
        val view = memoryView(id)
        val telling = text.takeIf { it.isNotEmpty() }?.let(view::tell)
        memoryBusy = true
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    h.memoryChatTell(id, text, files.mapNotNull { attachment(it) }.map { h.upload(it) })
                }
                readMemoryChat(id)
            } catch (e: ClientException) {
                memoryBusy = false
                telling?.let { view.telling.remove(it) }
                if (text.isNotEmpty()) view.draft.text = joined(view.draft.text, text)
                view.draft.files += files.filterNot { it in view.draft.files }
                notice = e.message
            }
        }
    }

    fun stopMemory(id: String) {
        memoryStopping = true
        act(failed = { memoryStopping = false }) { host?.memoryChatStop(id) }
    }

    /** An answer to the open memory chat's ask, read back after. */
    fun replyMemory(id: String, ask: String, yes: Boolean, answers: Map<String, String>, always: Boolean, failed: () -> Unit) {
        val s = session ?: return failed()
        act(then = { viewModelScope.launch { readMemoryChat(id) } }, failed = failed) { s.reply(ask, yes, answers, always) }
    }

    /** Refused, the switch reads what the host holds. */
    fun tempMemory(on: Boolean) {
        temps.values.forEach { it.acts++ }
        tempMemory = on
        act(failed = { readTemps() }) { host?.scratchpadMemory(on) }
    }

    private val chatFlow = MutableStateFlow<Chat?>(null)
    val chat: StateFlow<Chat?> = chatFlow

    /** On a call with the assistant, which covers the app. */
    var calling by mutableStateOf(false)

    /** The composer's recording under way: one at a time, on any place. */
    var dictating: Dictation? = null

    /** What is being read aloud, while it is. */
    var reading by mutableStateOf<Reading?>(null)

    private var session: Session? = null

    /** Whether the talk stays on its newest; anything the person sends brings it back. */
    val follow = Follow(holding = true)

    /** The message held, whose menu is open; the places blur under it. */
    var held by mutableStateOf<Held?>(null)

    /** The Work card held, whose menu is open; the places blur under it too. */
    var heldCard by mutableStateOf<HeldCard?>(null)

    /** The Work thread open over the places, by name. */
    var opened by mutableStateOf<String?>(null)
    private val threadViews = mutableMapOf<String, ThreadView>()

    /** Which Work sections the person has folded or opened, which stays on
     * this device; a section untouched lies as its `start` says. */
    private val sections = mutableStateMapOf<String, Boolean>()

    fun sectionOpen(title: String, start: Boolean) =
        sections[title] ?: prefs.getBoolean("work-open-${title.lowercase()}", start)

    fun flipSection(title: String, start: Boolean) {
        val open = !sectionOpen(title, start)
        sections[title] = open
        prefs.edit { putBoolean("work-open-${title.lowercase()}", open) }
    }

    /** The Workers tile's tab showing, `tree`, `watches` or `archived`; it
     * stays on this device (workTree.tsx `WorkersTile`). */
    var workersTab by mutableStateOf(prefs.getString("workers-tab", null)?.takeIf { it in WORKERS_TABS } ?: "tree")
        private set

    fun showWorkersTab(tab: String) {
        workersTab = tab
        prefs.edit { putString("workers-tab", tab) }
    }

    /** The voice picked since the app started, which the host now keeps
     * too (none until then: the host's `me.voice` is the saved one), and how
     * fast it speaks, which stays on this device. */
    var voice by mutableStateOf<String?>(null)
        private set
    var voiceSpeed by mutableStateOf(prefs.getFloat("voice-speed", 1f))
        private set

    /** How a call runs, kept on this device: "chat" (the chat answers,
     * spoken) or "grok" (Grok Voice in front, handing requests to the chat). */
    var callMode by mutableStateOf(prefs.getString("call-mode", "chat") ?: "chat")
        private set

    fun pickCallMode(m: String) {
        callMode = if (m == "grok") "grok" else "chat"
        prefs.edit { putString("call-mode", callMode) }
    }

    fun pickVoice(v: String) {
        voice = v
        act { host?.pickVoice(v) }
    }

    fun pickVoiceSpeed(s: Float) {
        voiceSpeed = (Math.round(s * 10) / 10f).coerceIn(0.5f, 3f)
        prefs.edit { putFloat("voice-speed", voiceSpeed) }
    }

    private val drafts = Drafts(app)

    /** Each draft in hand by its name on disk, for writing out as the app leaves. */
    private val keeping = mutableMapOf<String, Draft>()

    /** A draft as it stands on disk, written out soon after each change. */
    private fun kept(key: String): Draft {
        val d = drafts.read(key)
        keeping[key] = d
        viewModelScope.launch {
            snapshotFlow { d.kept() }.drop(1).collectLatest {
                delay(DRAFT_WRITE_MS)
                withContext(Dispatchers.IO) { drafts.write(key, it) }
            }
        }
        return d
    }

    /** The words and files being written, which a share from another app fills. */
    val draft = kept("chat")

    /** The Tasks sheet's words and files, kept as the chat's are until a
     * worker starts on them; `newTask` while the sheet is open. */
    val taskDraft = kept("task")
    var newTask by mutableStateOf(false)
    /** A pane asked for as a tab of its own (the composer's agents open a
     * worker's thread so): Places adds it to the bar and shows it. */
    var tabAsked by mutableStateOf<PhonePane?>(null)
    /** Counts asks for every worker (the composer tree's Show all, the
     * plus): Places lays the Work page over the item shown. */
    var allWorkersAsked by mutableIntStateOf(0)


    /** What another app shared into Libri: its text and files, into the composer. */
    /** A connection's sign-in finished from the address the server sent
     * the app back to; bumped on each, so Connections reloads. */
    var signedBack by mutableStateOf(0)
    /** What went wrong finishing the last sign-in, shown in Connections. */
    var signInSaid by mutableStateOf<String?>(null)

    private fun signedIn(back: Uri) {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val done = runCatching { h.serverFinish(back.toString()) }
            withContext(Dispatchers.Main) {
                signInSaid = done.exceptionOrNull()?.let { it.message }
                signedBack++
            }
        }
    }

    fun shared(intent: android.content.Intent) {
        val data = intent.data
        if (intent.action == android.content.Intent.ACTION_VIEW && data?.scheme == "is.libri" && data.path == "/oauth/back") {
            signedIn(data)
            return
        }
        if (intent.action != android.content.Intent.ACTION_SEND && intent.action != android.content.Intent.ACTION_SEND_MULTIPLE) return
        intent.getStringExtra(android.content.Intent.EXTRA_TEXT)?.let { draft.text = (draft.text + " " + it).trim() }
        val uris = if (intent.action == android.content.Intent.ACTION_SEND_MULTIPLE) {
            androidx.core.content.IntentCompat.getParcelableArrayListExtra(intent, android.content.Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
        } else {
            listOfNotNull(androidx.core.content.IntentCompat.getParcelableExtra(intent, android.content.Intent.EXTRA_STREAM, Uri::class.java))
        }
        uris.forEach { draft.files += picked(it, intent.type) }
        sharedPlace++
    }

    /** Bumped by each share, so the app shows the chat. */
    var sharedPlace by mutableStateOf(0)

    /** Photos this phone sent, by name: shown from the phone's own copy, so
     * none is asked of the host before its upload has landed. */
    val sentHere = mutableMapOf<String, String>()

    /** Where a file the host keeps is drawn from. */
    fun picture(name: String): String =
        sentHere[name] ?: session?.keptFile(name)?.let { "file://$it" } ?: host?.fileUrl(name).orEmpty()
    private var wasTyping = false
    private var workNews = 0uL
    private var tempNews = 0uL

    /** Tries the address typed; once the host there answers, adds it to
     * the hosts known and moves to it. */
    fun connect(typed: String) {
        viewModelScope.launch {
            connecting = true
            error = null
            try {
                val got = withContext(Dispatchers.IO) { probe(typed) }
                val whole = wholeAddress(typed)
                remember(KnownHost(got.id, got.name, whole))
                adding = false
                if (whole != address) use(whole)
                info = got
            } catch (e: ClientException) {
                error = e.message
            } finally {
                connecting = false
            }
        }
    }

    /** Opens the host entry over the host in use. */
    fun addHost() {
        error = null
        adding = true
    }

    /** Closes the host entry, back to the host in use as it was. */
    fun cancelAdding() {
        error = null
        adding = false
    }

    /** Moves to a known host (HostLink). */
    fun switchTo(url: String) {
        if (url == address) return
        use(url)
    }

    /** Takes a host off the list; the one in use stays, as on the page. */
    fun forgetHost(url: String) {
        if (url == address) return
        keepHosts(hosts.filter { it.url != url })
    }

    /** Asks every other known host who it is, timed, for the lists. */
    fun probeHosts() {
        val others = hosts.filter { it.url != address }
        viewModelScope.launch(Dispatchers.IO) {
            val got = others.map { h -> h to runCatching { probe(h.url) }.getOrNull() }
            withContext(Dispatchers.Main) {
                hostsSeen = got.associate { (h, i) -> h.url to i?.millis?.toLong() }
                // A host that answers names itself: keep its id and name current.
                got.forEach { (h, i) -> if (i != null && (i.id != h.id || i.name != h.name)) remember(KnownHost(i.id, i.name, h.url)) }
            }
        }
    }

    /** Leaves the host in use for the one at `whole`, starting clean. */
    private fun use(whole: String) {
        leave()
        prefs.edit { putString(ADDRESS, whole) }
        val picked = Host(whole)
        host = picked
        address = whole
        viewModelScope.launch {
            withContext(Dispatchers.IO) { Bell.follow(getApplication(), picked) }
            start()
            readTemps()
        }
    }

    /** Adds a host to the list, or updates the one there by id or address. */
    private fun remember(h: KnownHost) {
        val all = hosts.toMutableList()
        val at = all.indexOfFirst { (h.id.isNotEmpty() && it.id == h.id) || it.url == h.url }
        if (at < 0) all += h else all[at] = h
        if (all != hosts) keepHosts(all)
    }

    private fun keepHosts(all: List<KnownHost>) {
        hosts = all
        val list = JSONArray()
        all.forEach { list.put(JSONObject().put("id", it.id).put("name", it.name).put("url", it.url)) }
        prefs.edit { putString(HOSTS, list.toString()) }
    }

    private fun savedHosts(): List<KnownHost> {
        val list = runCatching { JSONArray(prefs.getString(HOSTS, null) ?: "[]") }.getOrDefault(JSONArray())
        val known = (0 until list.length()).map { list.getJSONObject(it) }
            .map { KnownHost(it.optString("id"), it.optString("name"), it.getString("url")) }
        if (known.isNotEmpty()) return known
        val one = prefs.getString(ADDRESS, null) ?: return emptyList()
        // Named by its address until it answers (refresh names it).
        val first = listOf(KnownHost("", one.substringAfter("://"), one))
        prefs.edit { putString(HOSTS, JSONArray().put(JSONObject().put("id", "").put("name", first[0].name).put("url", one)).toString()) }
        return first
    }

    /** Follows the host, if there is one and it isn't followed already. */
    fun start() {
        // The kinds of tile, for the bar's plus, names and menus (TileViews.kt).
        loadTiles()
        val at = address ?: return
        if (session != null) return
        val store = getApplication<Application>().filesDir.resolve("hosts").resolve(at.hashCode().toString())
        store.mkdirs()
        session = Session.open(at, store.path, object : ChatListener {
            override fun changed(chat: Chat) {
                // The host back after being away: what it said of itself is owed too.
                if (chat.link is Link.Live && chatFlow.value?.link !is Link.Live) refresh()
                chatFlow.value = chat
                threadPresentation = chat.workerMessages
                // A page published in the turn under way opens by itself, as an
                // artifact opens when it is made; only what the stream told live.
                chat.published?.let { pub ->
                    if (pub != published) {
                        published = pub
                        viewModelScope.launch(Dispatchers.Main) { opening = pub.page }
                    }
                }
                if (chat.workNews != workNews) {
                    workNews = chat.workNews
                    refreshWork()
                }
                if (chat.tempNews != tempNews) {
                    tempNews = chat.tempNews
                    readTemps()
                }
                // A turn's end moves the usage rings.
                if (wasTyping && !chat.typing) refreshUsage()
                wasTyping = chat.typing
            }
        })
    }

    /** What the composer and side panel show besides the talk. */
    /** Answers from a host already left are dropped, so a change of host
     * shows nothing of the last one. */
    fun refresh() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { h.info() }.onSuccess { i ->
                if (host === h) withContext(Dispatchers.Main) {
                    info = i
                    address?.let { remember(KnownHost(i.id, i.name, it)) }
                }
            }
            runCatching { h.me() }.onSuccess { if (host === h) me = it }
            runCatching { h.models() }.onSuccess { if (host === h) models = it }
        }
        refreshUsage()
    }

    private fun savedUsage(): Usage? = runCatching {
        val u = JSONObject(prefs.getString(USAGE, null) ?: return null)
        fun strings(a: JSONArray?) = (0 until (a?.length() ?: 0)).map { a!!.getString(it) }
        val accounts = u.getJSONArray("accounts")
        Usage(
            u.getBoolean("shown"),
            (0 until accounts.length()).map { i ->
                val a = accounts.getJSONObject(i)
                val windows = a.getJSONArray("windows")
                Account(
                    a.getString("account"), a.getString("name"),
                    (0 until windows.length()).map { j ->
                        val w = windows.getJSONObject(j)
                        Limit(
                            w.getString("window"), w.getString("name"), w.getDouble("pct"), w.getString("resets"),
                            w.optDouble("hours").takeIf { !it.isNaN() }, w.optLong("resetsAt", 0).takeIf { it > 0 },
                        )
                    },
                    strings(a.optJSONArray("notes")),
                    a.optString("plan").ifEmpty { null },
                    a.optLong("readAt", 0),
                    a.optBoolean("stale", false),
                )
            },
            strings(u.optJSONArray("notes")),
        )
    }.getOrNull()

    private fun keepUsage(u: Usage) {
        usage = u
        val accounts = JSONArray()
        u.accounts.forEach { a ->
            val windows = JSONArray()
            a.windows.forEach { windows.put(JSONObject().put("window", it.window).put("name", it.name).put("pct", it.pct).put("resets", it.resets).put("hours", it.hours).put("resetsAt", it.resetsAt)) }
            accounts.put(JSONObject().put("account", a.account).put("name", a.name).put("windows", windows).put("notes", JSONArray(a.notes)).put("plan", a.plan).put("readAt", a.readAt).put("stale", a.stale))
        }
        prefs.edit { putString(USAGE, JSONObject().put("shown", u.shown).put("accounts", accounts).put("notes", JSONArray(u.notes)).toString()) }
    }

    /** The subscription behind a composer's pick, while the rings are
     * shown; none for a model paid by key, or before the pick is known. */
    fun account(harness: String?, picked: String?): Account? {
        val u = usage?.takeIf { it.shown } ?: return null
        val id = models?.offers?.find { it.harness == harness && it.model == picked }?.usage ?: return null
        return u.accounts.find { it.account == id }
    }

    /** Shows the rings in every composer, or hides them, on the host for
     * every device. */
    fun showUsage(shown: Boolean) {
        val h = host ?: return
        usage?.let { keepUsage(it.copy(shown = shown)) }
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { h.showUsage(shown) }.onFailure { refreshUsage() }
        }
    }

    private fun refreshUsage() {
        val h = host ?: return
        viewModelScope.launch(Dispatchers.IO) { runCatching { h.usage() }.onSuccess { if (host === h) keepUsage(it) } }
    }

    /** Reads the conversation afresh: a call has gone into the history. */
    fun reconnect() {
        viewModelScope.launch {
            stop()
            start()
        }
    }

    /** Stops following the host: the app is out of view. */
    fun stop() {
        keeping.forEach { (key, d) -> drafts.write(key, d.kept()) }
        session?.let {
            it.detach()
            it.close()
        }
        session = null
    }

    /** Sends the person's words and files. One the host is down for waits
     * in the core's queue; one it refused is marked on the message. */
    fun send(text: String, files: List<Picked>, quote: Quote? = null) {
        val s = session ?: return
        // `/memory <name>` opens that memory's chat, as the web's tile does;
        // any other /memory goes on and is answered with a note.
        if (files.isEmpty()) memoryAsked(text, memories)?.let { id -> memoryTabAsked = id; return }
        follow.pinned = true
        files.filter { it.image }.forEach { sentHere[it.name] = it.uri.toString() }
        viewModelScope.launch(Dispatchers.IO) {
            try {
                s.send(text, files.mapNotNull { attachment(it) }, quote)
                // A command may have changed what the pickers show.
                if (text.startsWith("/")) refresh()
            } catch (e: ClientException) {
                notice = e.message
            }
        }
    }

    /** A refused message again, in its place; an edit's words, which the
     * core does not hold, go as a new message in place of the marked one. */
    fun resend(m: Message) {
        val s = session ?: return
        follow.pinned = true
        act {
            if (!s.retry(m.id)) {
                s.withdraw(m.id)
                s.send(m.text, emptyList(), m.quote)
            }
        }
    }

    /** A refused message taken out of the talk, its words and files back
     * in the box, after anything typed since. */
    fun putBack(m: Message) {
        val s = session ?: return
        viewModelScope.launch {
            val back = withContext(Dispatchers.IO) {
                s.withdraw(m.id)?.let { it.text to it.files.map(drafts::hold) }
            } ?: return@launch
            draft.text = joined(draft.text, back.first)
            draft.files += back.second
        }
    }

    /** The person's last message again: `edit` with new words, `again` to
     * draw its answer afresh. */
    fun redo(text: String, kind: String) {
        follow.pinned = true
        act { session?.redo(text, kind) }
    }

    fun stopTurn() = act { host?.stopTurn() }

    /** Answers an ask; `failed` frees its card to try again. A temp chat's
     * ask is read afresh after, as the core holds only the main chat's. */
    fun reply(ask: String, yes: Boolean, answers: Map<String, String>, temp: Boolean = false, always: Boolean = false, failed: () -> Unit = {}) {
        if (!temp) follow.pinned = true
        val s = session ?: return failed()
        act(then = { if (temp) readTemps() }, failed = failed) { s.reply(ask, yes, answers, always) }
    }

    fun decide(card: ULong, yes: Boolean) = act { host?.decide(card, yes) }

    fun pickModel(harness: String, model: String, temp: Boolean = false) = act(then = ::refresh) { host?.pickModel(harness, model, temp) }

    fun pickEffort(effort: String, temp: Boolean = false) = act(then = ::refresh) { host?.pickEffort(effort, temp) }

    fun pickAccess(access: String, temp: Boolean = false) = act(then = ::refresh) { host?.pickAccess(access, temp) }

    fun pickSandbox(sandbox: String, temp: Boolean = false) = act(then = ::refresh) { host?.pickSandbox(sandbox, temp) }

    /** What a harness starts on, in Settings. */
    fun pickAccessDefault(harness: String, access: String?, sandbox: String?) = act(then = ::refresh) { host?.pickAccessDefault(harness, access, sandbox) }

    /** One open thread's page, kept here by name. */
    fun threadView(name: String) = threadViews.getOrPut(name) { ThreadView(kept(threadKey(name))) }

    private fun threadKey(name: String) = "$THREAD_DRAFT${"$address $name".hashCode()}"

    /** Folds the open thread; it opens next at the latest. Its unsent words stay. */
    fun fold() {
        opened?.let { threadView(it).follow.pinned = true }
        opened = null
    }

    /** Asks for a thread's log. One at a time, so a slow answer can't put back an older log. */
    suspend fun readLog(name: String) {
        val h = host ?: return
        val view = threadView(name)
        view.reading.withLock {
            runCatching { withContext(Dispatchers.IO) { h.workLog(name) } }.onSuccess(view::heard)
        }
    }

    /** Words and files to a Work thread; the files go ahead, as the chat's do.
     * The words show at once; if the host refuses them, they and the files go
     * back in the box, after anything typed since. */
    fun tell(name: String, text: String, files: List<Picked>) {
        val view = threadView(name)
        val telling = text.takeIf { it.isNotEmpty() }?.let(view::tell)
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    val h = host ?: return@withContext
                    h.agentTell(name, text, files.mapNotNull { attachment(it) }.map { h.upload(it) })
                }
                readLog(name)
                refreshWork()
            } catch (e: ClientException) {
                telling?.let { view.telling.remove(it) }
                if (text.isNotEmpty()) view.draft.text = joined(view.draft.text, text)
                view.draft.files += files.filterNot { it in view.draft.files }
                notice = e.message
            }
        }
    }

    /** Runs a request off the main thread; a failure becomes the notice,
     * and `failed` undoes what the caller showed ahead of the answer. */
    fun act(then: () -> Unit = {}, failed: () -> Unit = {}, f: () -> Unit) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                f()
                withContext(Dispatchers.Main) { then() }
            } catch (e: ClientException) {
                withContext(Dispatchers.Main) {
                    notice = e.message
                    failed()
                }
            }
        }
    }

    /** Drops all the host in use showed, before another is used. */
    private fun leave() {
        stop()
        hush(this)
        info = null
        host = null
        memory = null
        silent = null
        models = null
        opened = null
        threadViews.clear()
        keeping.keys.removeAll { it.startsWith(THREAD_DRAFT) }
        me = null
        voice = null
        usage = null
        prefs.edit { remove(USAGE) }
        threads = emptyList()
        talk = emptyList()
        memories = emptyList()
        library = null
        watches = emptyList()
        temps.values.forEach { it.forget() }
        follow.pinned = true
        chatFlow.value = null
        threadPresentation = emptyMap()
    }

    override fun onCleared() = stop()

    /** A chosen file's bytes, and for a photo too big or of a kind the
     * model can't see, a JPEG no longer than 2048 px (chat.ts `fit`). */
    private fun attachment(p: Picked): Attachment? {
        val resolver = getApplication<Application>().contentResolver
        val data = runCatching { resolver.openInputStream(p.uri)?.use { it.readBytes() } }.getOrNull() ?: return null
        val view = if (p.image && (data.size > PHOTO_MAX || p.mime !in PHOTO_TYPES)) {
            runCatching {
                val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(data)) { decoder, info, _ ->
                    val long = maxOf(info.size.width, info.size.height)
                    if (long > 2048) {
                        val scale = 2048f / long
                        decoder.setTargetSize((info.size.width * scale).toInt(), (info.size.height * scale).toInt())
                    }
                }
                ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, 85, it) }.toByteArray()
            }.getOrNull()
        } else {
            null
        }
        return Attachment(p.name, p.mime, data, view)
    }

    /** A chosen file as the composer holds it, by its name and kind. */
    fun picked(uri: Uri, declared: String? = null): Picked {
        val resolver = getApplication<Application>().contentResolver
        val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        } ?: uri.lastPathSegment ?: "file"
        val mime = resolver.getType(uri) ?: declared?.takeIf { '*' !in it } ?: "application/octet-stream"
        return Picked(uri, name, mime)
    }

    companion object {
        const val ADDRESS = "address"
        const val HOSTS = "hosts"
        const val DRAFT_WRITE_MS = 300L
        const val THREAD_DRAFT = "thread-"
        // A new name, as what is kept under it changed shape.
        const val USAGE = "usage-accounts"
        const val TEXT_MIN = 12
        const val TEXT_MAX = 24
        const val PHOTO_MAX = 3_500_000
        const val TEMP_READ_MS = 3_000L
        val PHOTO_TYPES = setOf("image/jpeg", "image/png", "image/gif", "image/webp")
    }
}

/** A host this phone knows (hosts.ts Host): the id and name it last gave,
 * empty until it answers, and its address. */
data class KnownHost(val id: String, val name: String, val url: String)
