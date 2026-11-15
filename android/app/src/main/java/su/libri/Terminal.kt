package su.libri

import android.annotation.SuppressLint
import android.content.ClipboardManager
import android.content.Context
import android.text.InputType
import android.util.Base64
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.WebViewAssetLoader
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONObject

/** Modifiers the keys row holds for the next key, typed or tapped. */
private data class Mods(val ctrl: Boolean = false, val alt: Boolean = false, val shift: Boolean = false) {
    val any get() = ctrl || alt || shift
}

private val ARROWS = mapOf("↑" to "A", "↓" to "B", "→" to "C", "←" to "D")

/** What a key sends with the modifiers held, as xterm encodes it (terminal.tsx). */
private fun encode(key: String, m: Mods): String {
    val code = 1 + (if (m.shift) 1 else 0) + (if (m.alt) 2 else 0) + (if (m.ctrl) 4 else 0)
    ARROWS[key]?.let { return if (code > 1) "\u001b[1;$code$it" else "\u001b[$it" }
    return when (key) {
        "Tab" -> if (m.shift) "\u001b[Z" else "\t"
        "Esc" -> "\u001b"
        else -> "\r"
    }
}

/** A typed character with the held modifiers: Shift uppercases, Ctrl makes
 * its control code, Alt puts Escape before it. */
private fun modify(typed: String, m: Mods): String {
    var data = if (m.shift) typed.uppercase() else typed
    if (m.ctrl) {
        val c = data.uppercase()[0].code
        data = if (c in 64..95) (c - 64).toChar().toString() else if (data == " ") "\u0000" else data
    }
    return if (m.alt) "\u001b$data" else data
}

private enum class Shell { CONNECTING, OPEN, ENDED, LOST }


/** The host's shell, an item on the bar (Places.kt), with the row of keys a
 * phone keyboard lacks above the keyboard, as Paseo, Termux and Blink lay
 * it out. Its place keeps it clear of the status bar, the bar and the
 * keyboard. The shell lives on the host and goes on while this is closed;
 * opening it again is sent the screen as it is. */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun TerminalScreen(model: ChatModel, id: String? = null) {
    val context = LocalContext.current
    val host = model.host ?: return
    var state by remember { mutableStateOf(Shell.CONNECTING) }
    var mods by remember { mutableStateOf(Mods()) }
    var run by remember { mutableStateOf(0) }
    val p = Ink
    val web = remember {
        TerminalView(context).apply {
            settings.javaScriptEnabled = true
            setBackgroundColor(p.paper.toArgb())
        }
    }
    // A terminal added beside one is a shell of its own (terminal.rs names
    // them by `?id=`); none, the shared one.
    val link = remember { TerminalLink(host.terminalUrl() + (id?.let { "?id=" + java.net.URLEncoder.encode(it, "UTF-8") } ?: ""), web) }
    // Restart shell from the item's menu: the clean close it ends in opens
    // a new shell rather than offering one.
    var restarting by remember { mutableStateOf(false) }
    link.onState = { if (it == Shell.ENDED && restarting) { restarting = false; run++ } else state = it }
    DisposableEffect(link) {
        model.terminalActions[id ?: ""] = TerminalActions(
            paste = { clipboard -> link.paste(clipboard) },
            clear = { link.send("\u000c") },
            restart = { restarting = true; link.restart() },
        )
        onDispose { model.terminalActions.remove(id ?: "") }
    }
    link.mods = { mods }
    link.release = { mods = Mods() }
    DisposableEffect(run, p) {
        // The page's colours (terminal.tsx theme), so the terminal sits in it.
        val theme = JSONObject()
            .put("background", hex(p.paper))
            .put("foreground", hex(p.ink))
            .put("cursor", hex(p.accent))
            .put("cursorAccent", hex(p.paper))
            .put("selectionBackground", hex(p.hover))
            .toString()
        web.setBackgroundColor(p.paper.toArgb())
        link.start(theme)
        onDispose { link.stop() }
    }
    Column(Modifier.fillMaxSize().background(p.paper)) {
        // No head and no ✕: the bar names the terminal
        // and leaves it; the shell's state floats at the top right only while
        // it is not plainly running (terminal.tsx).
        Box(Modifier.weight(1f).fillMaxWidth().padding(top = 6.dp)) {
            AndroidView(factory = { web }, modifier = Modifier.fillMaxSize())
            if (state != Shell.OPEN) Box(Modifier.align(Alignment.TopEnd).padding(end = 14.dp, top = 4.dp)) {
                if (state == Shell.ENDED) FloatPill(onClick = { run++ }) { Text("Shell exited · New shell", style = type(TypeScale.label, 1.3f), color = p.ink) }
                else FloatPill(onClick = null) { Text(if (state == Shell.CONNECTING) "Connecting…" else "Reconnecting…", style = type(TypeScale.label, 1.3f), color = p.mid) }
            }
        }
        Keys(mods, onMod = { mods = it }) { key ->
            when (key) {
                "hide" -> {
                    context.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(web.windowToken, 0)
                    // Focus given back raises the keyboard again, and focus
                    // left where it is would keep a tap from raising it.
                    link.blur()
                    return@Keys
                }
                "Paste" -> context.getSystemService(ClipboardManager::class.java).primaryClip?.getItemAt(0)?.coerceToText(context)?.let {
                    link.paste(it.toString())
                }
                else -> {
                    link.send(encode(key, mods))
                    mods = Mods()
                }
            }
            link.focus()
        }
    }
}

/** A keyboard that composes holds a word back until it is done with it and
 * corrects it on the way; a shell wants each key as it is struck, and a
 * password typed here should stay out of what the keyboard learns. So the
 * field is declared as Termux declares its own. */
private class TerminalView(context: Context) : WebView(context) {
    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        val connection = super.onCreateInputConnection(outAttrs)
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        return connection
    }

    // The keyboard rising or falling resizes this view: the page fits its
    // grid to the new room, so the last rows and the prompt stay above it.
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        evaluateJavascript("window.libri && libri.fit()", null)
    }
}

private fun hex(c: Color) = String.format("#%06X", c.toArgb() and 0xFFFFFF)

/** The keyboard put away: a keyboard struck through (terminal.tsx). */
private val hideGlyph = run {
    androidx.compose.ui.graphics.vector.ImageVector.Builder("hide", 24.dp, 24.dp, 24f, 24f).apply {
        listOf(
            "M4.5 6h15a1.5 1.5 0 0 1 1.5 1.5v9a1.5 1.5 0 0 1-1.5 1.5h-15a1.5 1.5 0 0 1-1.5-1.5v-9a1.5 1.5 0 0 1 1.5-1.5z",
            "M7 10h.01M11 10h.01M15 10h.01M8 14h8M3 3l18 18",
        ).forEach {
            addPath(
                androidx.compose.ui.graphics.vector.PathParser().parsePathString(it).toNodes(),
                stroke = androidx.compose.ui.graphics.SolidColor(Color.Black),
                strokeLineWidth = 1.8f,
                strokeLineCap = androidx.compose.ui.graphics.StrokeCap.Round,
                strokeLineJoin = androidx.compose.ui.graphics.StrokeJoin.Round,
            )
        }
    }.build()
}

/** The keys a phone keyboard lacks, as the page lays them: six across in
 * two rows, 40px tall, --block with a hairline; a held modifier in ink. */
@Composable
private fun Keys(mods: Mods, onMod: (Mods) -> Unit, tap: (String) -> Unit) {
    val p = Ink
    val rows = listOf(
        listOf("Esc", "Tab", "Ctrl", "↑", "Shift", "hide"),
        listOf("Alt", "Paste", "←", "↓", "→", "Enter"),
    )
    Column(Modifier.fillMaxWidth().background(p.paper).padding(horizontal = 8.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        rows.forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                row.forEach { k ->
                    val on = (k == "Ctrl" && mods.ctrl) || (k == "Alt" && mods.alt) || (k == "Shift" && mods.shift)
                    Box(
                        Modifier.weight(1f).height(40.dp).clip(RoundedCornerShape(Radii.lg))
                            .background(if (on) p.ink else p.block).border(1.dp, p.hair, RoundedCornerShape(Radii.lg))
                            .clickable {
                                when (k) {
                                    "Ctrl" -> onMod(mods.copy(ctrl = !mods.ctrl))
                                    "Alt" -> onMod(mods.copy(alt = !mods.alt))
                                    "Shift" -> onMod(mods.copy(shift = !mods.shift))
                                    else -> tap(k)
                                }
                            },
                        contentAlignment = Alignment.Center,
                    ) {
                        if (k == "hide") {
                            Glyph(hideGlyph, 18.dp, p.mid, label = "Hide the keyboard")
                        } else {
                            Text(k, style = type(TypeScale.body, 1.2f), color = if (on) p.paper else p.mid)
                        }
                    }
                }
            }
        }
    }
}

/** The socket to the host's shell and the page that draws it. The socket
 * is the app's, not the page's: the host takes a terminal only from its
 * own pages or from a client that names no page (terminal.rs). */
private class TerminalLink(private val url: String, private val web: WebView) {
    var onState: (Shell) -> Unit = {}
    var mods: () -> Mods = { Mods() }
    var release: () -> Unit = {}
    private val client = OkHttpClient()
    private var socket: WebSocket? = null
    private var size = "{\"cols\":80,\"rows\":24}"
    private var stopped = false
    private var first = true

    fun start(theme: String) {
        stopped = false
        val loader = WebViewAssetLoader.Builder().addPathHandler("/assets/", WebViewAssetLoader.AssetsPathHandler(web.context)).build()
        web.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                loader.shouldInterceptRequest(request.url)
        }
        web.removeJavascriptInterface("Libri")
        web.addJavascriptInterface(Page(theme), "Libri")
        web.loadUrl("https://appassets.androidplatform.net/assets/terminal/index.html")
    }

    fun stop() {
        stopped = true
        socket?.close(1000, null)
        socket = null
    }

    /** Asks the host to hang the shell up (terminal.rs `RESTART`); its
     * clean close then opens a new one. */
    fun restart() {
        socket?.send("""{"op":"restart"}""")
    }

    fun send(data: String) {
        socket?.send(data.toByteArray().toByteString())
    }

    fun paste(text: String) = js("libri.paste(${JSONObject.quote(text)})")

    fun focus() = js("libri.focus()")

    fun blur() = js("libri.blur()")

    private fun js(code: String) = web.post { web.evaluateJavascript(code, null) }

    private fun connect() {
        if (stopped) return
        onState(Shell.CONNECTING)
        first = true
        socket = client.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                webSocket.send(size)
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                val b64 = Base64.encodeToString(bytes.toByteArray(), Base64.NO_WRAP)
                if (first) {
                    first = false
                    onState(Shell.OPEN)
                    js("libri.screen('$b64')")
                } else {
                    js("libri.output('$b64')")
                }
            }

            // The host closes cleanly when the shell exits; anything else
            // is the link lost, tried again.
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                if (!stopped) web.post { onState(Shell.ENDED) }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                if (stopped || socket !== webSocket) return
                web.post { onState(Shell.LOST) }
                web.postDelayed({ connect() }, 2000)
            }
        })
    }

    private inner class Page(private val theme: String) {
        @JavascriptInterface fun theme() = theme

        @JavascriptInterface fun ready(cols: Int, rows: Int) {
            size = "{\"cols\":$cols,\"rows\":$rows}"
            // The keyboard waits for a tap: on a phone it would cover half
            // the screen of a terminal opened only to read it.
            web.post { connect() }
        }

        @JavascriptInterface fun sized(cols: Int, rows: Int) {
            size = "{\"cols\":$cols,\"rows\":$rows}"
            socket?.send(size)
        }

        // Modifiers from the keys row apply to the next character, as the
        // keyboard's own would.
        @JavascriptInterface fun typed(data: String) {
            val m = mods()
            if (m.any && data.length == 1) {
                send(modify(data, m))
                web.post { release() }
            } else {
                send(data)
            }
        }
    }
}

/** What an item's menu does to a terminal (tiles.rs rows paste, clear,
 * restart), by its shell's name, "" the shared one. */
class TerminalActions(val paste: (String) -> Unit, val clear: () -> Unit, val restart: () -> Unit)
