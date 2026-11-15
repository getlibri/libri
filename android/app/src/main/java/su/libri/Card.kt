package su.libri

import android.annotation.SuppressLint
import android.graphics.Color as AndroidColor
import android.os.Message
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject
import su.libri.core.DrawnCard
import su.libri.core.ToolCall
import su.libri.core.drawnCard

/** A host's page in a web view filling the screen: one call's card made
 * large (`#call-card=…&full=1`), which hosts the card as the web does, so
 * the app carries no MCP Apps host of its own. Closing the card, or Back, comes
 * back to the chat. */
@SuppressLint("SetJavaScriptEnabled", "RequiresFeature")
@Composable
fun CardScreen(at: String, close: () -> Unit) {
    BackHandler(onBack = close)
    val closing by rememberUpdatedState(close)
    AndroidView(
        factory = { c ->
            WebView(c).apply {
                settings.javaScriptEnabled = true
                // The site keeps its theme and hosts in local storage.
                settings.domStorageEnabled = true
                webViewClient = HostOnly(at)
                webChromeClient = object : WebChromeClient() {
                    // A tool's card closes with window.close().
                    override fun onCloseWindow(window: WebView) = closing()
                    override fun onCreateWindow(view: WebView, dialog: Boolean, user: Boolean, result: Message?) = false
                }
                // A call's card made large closes by its page's word.
                listen(this, at) { said -> if (said.optBoolean("close")) closing() }
                loadUrl(at)
            }
        },
        onRelease = { it.destroy() },
        modifier = Modifier.fillMaxSize().background(Ink.paper).systemBarsPadding().imePadding(),
    )
}

/** Keeps a card's web view on the host's page: a link it opens elsewhere
 * (the card's `ui/open-link`, which the page opens as a new window) goes to
 * the browser, as on the web, never into the card's place. */
private class HostOnly(at: String) : WebViewClient() {
    private val host = android.net.Uri.parse(at).let { "${it.scheme}://${it.authority}" }

    override fun shouldOverrideUrlLoading(view: WebView, request: android.webkit.WebResourceRequest): Boolean {
        val to = request.url
        if ("${to.scheme}://${to.authority}" == host) return false
        if (to.scheme == "http" || to.scheme == "https") {
            runCatching { view.context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, to).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
        return true
    }
}

/** What a chat gives the cards its calls draw (MCP Apps): which calls have
 * one, the host's page of a call's card, where a card's message goes, and
 * how a card fills the screen. */
data class CardHost(
    val cards: Set<String>,
    /** Of those, the tools whose card opens by itself (apps.rs `Card::opens`). */
    val opens: Set<String>,
    val url: (drawn: DrawnCard, call: ToolCall, fresh: Boolean, full: Boolean) -> String?,
    val say: (String) -> Unit,
    val full: (String) -> Unit,
)

/** The assistant messages of turns the person's own words started (the
 * core's `personTurns`), whose cards may open by themselves. */
val LocalPersonTurns = compositionLocalOf<Set<String>> { emptySet() }

val LocalCardHost = compositionLocalOf<CardHost?> { null }

/** The cards a message's calls drew, under its calls and outside their
 * fold, as the web draws them (App.tsx CallCard). A card shows only when
 * showing it is the point: the model asked (`show_card`, drawn from a fresh
 * call), or the tool's card is what it is for (`opens`) and the person's
 * own turn made the call: heard as it ran, it opens once it has an answer;
 * read back from the record, it waits behind a pill. Any other call is its
 * row alone, whose Open shows its card on asking. */
@Composable
fun CallCards(calls: List<ToolCall>, byPerson: Boolean) {
    val host = LocalCardHost.current ?: return
    calls.forEach { call ->
        val drawn = remember(call.name, call.arguments) { drawnCard(call.name, call.arguments) }
        if (drawn.name !in host.cards) return@forEach
        when {
            drawn.shown -> if (call.done) CallCard(drawn, call, fresh = true, host)
            drawn.name !in host.opens || !byPerson -> Unit
            call.heard -> if (call.done) CallCard(drawn, call, fresh = false, host)
            else -> {
                var shown by rememberSaveable(call.id) { mutableStateOf(false) }
                if (shown) CallCard(drawn, call, fresh = true, host) else ShowCard(drawn) { shown = true }
            }
        }
    }
}

/** The way to a kept call's card (App.tsx CallCardOf's pill). */
@Composable
private fun ShowCard(drawn: DrawnCard, show: () -> Unit) {
    val p = Ink
    Row(
        Modifier
            .padding(vertical = 6.dp)
            .clip(RoundedCornerShape(50))
            .border(1.dp, p.hair, RoundedCornerShape(50))
            .clickable(onClick = show)
            .padding(horizontal = 12.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Glyph(Glyph.panel, 14.dp, p.mid)
        Text("Show the ${serverOf(drawn.name)} card", style = Type.label, color = p.mid)
    }
}

/** The server a call's tool belongs to, as harnesses name their calls
 * (apps.rs call_names): `mcp__server__tool`, or `server_tool`. */
private fun serverOf(name: String): String =
    if (name.startsWith("mcp__")) name.removePrefix("mcp__").substringBefore("__") else name.substringBefore('_')

/** One call's card in the chat: the host's page of it (`#call-card=`) in a
 * web view as tall as the card says it is. */
@SuppressLint("SetJavaScriptEnabled", "RequiresFeature")
@Composable
private fun CallCard(drawn: DrawnCard, call: ToolCall, fresh: Boolean, host: CardHost) {
    val p = Ink
    val at = host.url(drawn, call, fresh, false)?.let { hostLook(it, p) } ?: return
    // Kept across a scroll away and back, so the card returns at its size.
    var tall by rememberSaveable(call.id) { mutableIntStateOf(FIRST) }
    val say by rememberUpdatedState(host.say)
    val full by rememberUpdatedState(host.full)
    AndroidView(
        factory = { c ->
            WebView(c).apply {
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                webViewClient = HostOnly(at)
                // The chat's paper shows round the card's frame.
                setBackgroundColor(AndroidColor.TRANSPARENT)
                listen(this, at) { said ->
                    when {
                        said.has("height") -> tall = said.optInt("height").coerceIn(1, TALLEST)
                        said.has("say") -> said.optString("say").takeIf { it.isNotBlank() }?.let(say)
                        said.optBoolean("full") -> full(hostLook(host.url(drawn, call, fresh, true) ?: return@listen, p))
                    }
                }
                loadUrl(at)
            }
        },
        update = { if (it.url != at) it.loadUrl(at) },
        onRelease = { it.destroy() },
        modifier = Modifier.fillMaxWidth().height(tall.dp),
    )
}

/** How tall a card stands before it says (card.tsx FIRST, with its frame). */
private const val FIRST = 140

/** The tallest a card stands in the chat (card.tsx TALLEST, with its head). */
private const val TALLEST = 700

/** Hears what the host's page at `at` says as JSON through `LibriCard`,
 * from the host's own origin only, never from a card's frames, whose origin
 * is opaque. */
@SuppressLint("RequiresFeature")
private fun listen(web: WebView, at: String, heard: (JSONObject) -> Unit) {
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) return
    val origin = android.net.Uri.parse(at).let { "${it.scheme}://${it.authority}" }
    WebViewCompat.addWebMessageListener(web, "LibriCard", setOf(origin)) { _, message, _, main, _ ->
        if (main) message.data?.let { runCatching { JSONObject(it) }.getOrNull() }?.let(heard)
    }
}
