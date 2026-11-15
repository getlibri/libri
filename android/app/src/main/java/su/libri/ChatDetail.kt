package su.libri

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import coil3.compose.AsyncImage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONObject

val LocalDisplayFile = staticCompositionLocalOf<(String) -> String> { { it } }

val LocalChatDetail = staticCompositionLocalOf { "standard" }

@Composable
internal fun Metrics(raw: String) {
    val value = remember(raw) { runCatching { JSONObject(raw) }.getOrNull() } ?: return
    val tokens = value.optDouble("context_tokens", Double.NaN)
    val window = value.optDouble("context_window", Double.NaN)
    val speed = value.optDouble("tokens_per_second", Double.NaN)
    Column(Modifier.padding(vertical = 8.dp)) {
        if (tokens.isFinite() && window.isFinite() && window > 0) {
            Text("Context ${"%,d".format(tokens.toLong())} / ${"%,d".format(window.toLong())}", style = Type.note, color = Ink.soft)
            LinearProgressIndicator(progress = { (tokens / window).toFloat().coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth())
        }
        if (speed.isFinite()) Text("${"%.1f".format(speed)} tok/s", style = Type.note, color = Ink.soft)
    }
}

/** The same structured result fields as the web renderer; no text reparsing. */
@Composable
internal fun RichResult(raw: String) {
    val result = remember(raw) { runCatching { JSONObject(raw) }.getOrNull() } ?: return
    val details = result.optJSONObject("details")
    val files = details?.optJSONArray("files")
    val changes = if (files != null) (0 until files.length()).mapNotNull { files.optJSONObject(it) } else listOfNotNull(details)
    changes.forEach { file ->
        val diff = file.optString("diff").ifBlank {
            if (file.has("oldText") && file.has("newText")) "- ${file.optString("oldText")}\n+ ${file.optString("newText")}" else ""
        }
        if (diff.isNotBlank()) {
            Text(file.optString("path"), style = Type.note, color = Ink.soft)
            Column(Modifier.fillMaxWidth().background(Ink.block).horizontalScroll(rememberScrollState()).padding(10.dp)) {
                diff.lines().forEach { line ->
                    Text(line.ifEmpty { " " }, style = Type.code, color = Ink.ink,
                        modifier = Modifier.fillMaxWidth().background(when {
                            line.startsWith("+") -> androidx.compose.ui.graphics.Color(0x2026734d)
                            line.startsWith("-") -> androidx.compose.ui.graphics.Color(0x20b74747)
                            else -> androidx.compose.ui.graphics.Color.Transparent
                        }))
                }
            }
        }
    }
    result.optJSONArray("content")?.let { content ->
        (0 until content.length()).forEach { i ->
            val image = content.optJSONObject(i) ?: return@forEach
            val mime = image.optString("mimeType")
            if (image.optString("type") == "image" && mime in listOf("image/png", "image/jpeg", "image/gif", "image/webp")) {
                val bytes = remember(raw, i) { runCatching { android.util.Base64.decode(image.optString("data"), android.util.Base64.DEFAULT) }.getOrNull() }
                AsyncImage(model = image.optString("file").takeIf { it.isNotBlank() }?.let(LocalDisplayFile.current) ?: bytes, contentDescription = "Tool image", modifier = Modifier.fillMaxWidth().heightIn(max = 320.dp))
            }
        }
    }
}

@Composable
internal fun OmpSignIn(model: ChatModel) {
    val context = LocalContext.current
    var provider by remember(model.host) { mutableStateOf<String?>(null) }
    var state by remember(model.host) { mutableStateOf<JSONObject?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var starting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    Head("omp accounts", Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp))
    listOf("openai-codex-device" to "ChatGPT", "kimi-code" to "Kimi Code", "kilo" to "Kilo").forEach { (id, title) ->
        SettingRow("Sign in with $title", chevron = false, onClick = {
            if (!starting && provider == null) {
                starting = true
                scope.launch {
                    runCatching { withContext(Dispatchers.IO) { JSONObject(model.host!!.ompSignIn(id)) } }
                        .onSuccess { state = it; provider = id; error = null }
                        .onFailure { error = it.message }
                    starting = false
                }
            }
        })
    }
    LaunchedEffect(model.host) {
        val previous = runCatching { withContext(Dispatchers.IO) { JSONObject(model.host!!.signInProgress()).optJSONObject("omp") } }.getOrNull()
        if (previous?.optString("state") == "waiting") {
            state = previous
            provider = previous.optString("provider")
        }
    }
    LaunchedEffect(provider) {
        if (provider == null) return@LaunchedEffect
        while (true) {
            delay(1500)
            val next = runCatching { withContext(Dispatchers.IO) { JSONObject(model.host!!.signInProgress()).optJSONObject("omp") } }.getOrNull() ?: continue
            state = next
            if (next.optString("state") in listOf("done", "failed")) { provider = null; break }
        }
    }
    if (starting) Small("Starting sign-in…", Modifier.padding(horizontal = 20.dp))
    state?.let { s ->
        val status = s.optString("state", "waiting")
        Small(when (status) { "done" -> "Signed in. Credentials stay in omp."; "failed" -> s.optString("error", "Sign-in failed"); else -> "Approve the code in your browser: ${s.optString("code")}" }, Modifier.padding(horizontal = 20.dp))
        if (status == "waiting" && s.optString("url").isNotBlank()) SettingRow("Open sign-in page", onClick = {
            val uri = Uri.parse(s.optString("url"))
            if (uri.scheme == "https") context.startActivity(Intent(Intent.ACTION_VIEW, uri))
        })
    }
    error?.let { Small(it, Modifier.padding(horizontal = 20.dp)) }
    Small("Copilot, xAI, GLM and Meta use omp login in the host terminal. Meta subscriptions use Muse. Claude and Google subscriptions stay in their own harnesses.", Modifier.padding(horizontal = 20.dp))
}
