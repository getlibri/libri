package su.libri

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.Build
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import su.libri.core.ClientException

/** How long a failure stays over the composer. */
private const val SHOWN_MS = 4000L

/** `words` after what is already in the box. */
fun joined(text: String, words: String) = if (text.isEmpty() || text.last().isWhitespace()) text + words else "$text $words"

enum class Mic { IDLE, HEARING, WRITING }

/**
 * Dictation, as dictate.tsx has it: tap, speak, tap again, and the words join
 * the composer to be read over before they go. The recording goes to the
 * host, which writes it down with the engine chosen in Settings, Voice;
 * neither keeps it, nor does this file outlast it. Or, chosen there too,
 * the phone's own recognizer writes it down, showing the words as they
 * come. The microphone is held only between the two taps.
 */
class Dictation(private val context: Context, private val model: ChatModel, private val scope: CoroutineScope) {
    var mic by mutableStateOf(Mic.IDLE)
        private set
    var failed by mutableStateOf<String?>(null)
    /** The phone recognizer's words so far. */
    var live by mutableStateOf("")
        private set
    var add: (String) -> Unit = {}
    private var recorder: MediaRecorder? = null
    private var file: File? = null
    private var recognizer: SpeechRecognizer? = null

    private fun fail(why: String) {
        mic = Mic.IDLE
        failed = why
    }

    /** Starts hearing, stopping anything read aloud, which the mic would
     * hear, and any other composer's recording. */
    fun start() {
        hush(model)
        model.dictating?.stop()
        failed = null
        if (model.dictateOnPhone) return startOnPhone()
        val into = File.createTempFile("dictation", ".m4a", context.cacheDir)
        val r = MediaRecorder(context)
        try {
            r.setAudioSource(MediaRecorder.AudioSource.VOICE_RECOGNITION)
            r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            r.setAudioChannels(1)
            r.setAudioSamplingRate(16_000)
            r.setAudioEncodingBitRate(32_000)
            r.setOutputFile(into)
            r.prepare()
            r.start()
        } catch (e: Exception) { // MediaRecorder throws IOException and bare RuntimeExceptions.
            android.util.Log.w("libri", "dictate: $e")
            r.release()
            into.delete()
            return fail("The microphone is blocked")
        }
        recorder = r
        file = into
        model.dictating = this
        mic = Mic.HEARING
    }

    /** The phone's recognizer, on the device itself where Android offers
     * that; it ends by itself after a long pause. */
    private fun startOnPhone() {
        val onDevice = Build.VERSION.SDK_INT >= 33 && SpeechRecognizer.isOnDeviceRecognitionAvailable(context)
        if (!onDevice && !SpeechRecognizer.isRecognitionAvailable(context)) return fail("This phone has no speech recognizer")
        val r = if (onDevice) SpeechRecognizer.createOnDeviceSpeechRecognizer(context) else SpeechRecognizer.createSpeechRecognizer(context)
        val first = { b: Bundle? -> b?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull().orEmpty() }
        r.setRecognitionListener(object : RecognitionListener {
            override fun onPartialResults(b: Bundle?) {
                live = first(b)
            }
            override fun onResults(b: Bundle?) {
                val words = first(b).trim()
                endOnPhone()
                if (words.isEmpty()) return fail("Nothing heard")
                mic = Mic.IDLE
                add(words)
            }
            override fun onError(error: Int) {
                endOnPhone()
                fail(if (error == SpeechRecognizer.ERROR_NO_MATCH || error == SpeechRecognizer.ERROR_SPEECH_TIMEOUT) "Nothing heard" else "Couldn't write that down")
            }
            override fun onReadyForSpeech(b: Bundle?) {}
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(db: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {}
            override fun onEvent(type: Int, b: Bundle?) {}
        })
        val asked = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            .putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
        live = ""
        recognizer = r
        model.dictating = this
        mic = Mic.HEARING
        r.startListening(asked)
    }

    private fun endOnPhone() {
        recognizer?.destroy()
        recognizer = null
        live = ""
        if (model.dictating === this) model.dictating = null
    }

    /** Lets go of the microphone and has what it heard written down. */
    fun stop() {
        recognizer?.let {
            mic = Mic.WRITING
            return it.stopListening()
        }
        val r = recorder ?: return
        val from = file ?: return
        recorder = null
        if (model.dictating === this) model.dictating = null
        mic = Mic.WRITING
        // Not left to a scope that may end first: the microphone must go.
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            val audio = withContext(Dispatchers.IO + NonCancellable) { finish(r, from) }
            if (audio == null) return@launch fail("Nothing heard")
            try {
                val words = withContext(Dispatchers.IO) { model.host?.transcribe(audio, "audio/mp4") }.orEmpty().trim()
                if (words.isEmpty()) return@launch fail("Nothing heard")
                mic = Mic.IDLE
                add(words)
            } catch (e: ClientException) {
                android.util.Log.w("libri", "dictate: $e")
                fail("Couldn't write that down")
            }
        }
    }

    /** Drops a recording no composer is left to take. */
    fun drop() {
        recognizer?.let { endOnPhone(); mic = Mic.IDLE }
        val r = recorder ?: return
        val from = file ?: return
        recorder = null
        if (model.dictating === this) model.dictating = null
        scope.launch(start = CoroutineStart.UNDISPATCHED) { withContext(Dispatchers.IO + NonCancellable) { finish(r, from) } }
    }

    /** The recording's bytes; none when it held nothing, which `stop` throws on. */
    private fun finish(r: MediaRecorder, from: File): ByteArray? {
        val ended = runCatching { r.stop() }.isSuccess
        r.release()
        val audio = if (ended) runCatching { from.readBytes() }.getOrNull() else null
        from.delete()
        return audio?.takeIf { it.isNotEmpty() }
    }
}

/** The microphone button, filled in while it hears; `add` takes the words.
 * `top` is where the composer's card starts, in the window. */
@Composable
fun Dictate(model: ChatModel, top: Int, add: (String) -> Unit) {
    val p = Ink
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val taking by rememberUpdatedState(add)
    val d = remember { Dictation(context, model, scope) }
    d.add = { taking(it) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { yes ->
        if (yes) d.start() else d.failed = "The microphone is blocked"
    }
    DisposableEffect(Unit) { onDispose { d.drop() } }
    // Android silences a microphone left in the background.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { d.stop() }
    LaunchedEffect(d.failed) {
        if (d.failed != null) {
            delay(SHOWN_MS)
            d.failed = null
        }
    }
    val hearing = d.mic == Mic.HEARING
    val writing = d.mic == Mic.WRITING
    Box {
        (d.failed ?: d.live.ifEmpty { null })?.let { Failed(it, top, live = d.failed == null) }
        Round(
            onClick = {
                when {
                    hearing -> d.stop()
                    context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED -> d.start()
                    else -> ask.launch(Manifest.permission.RECORD_AUDIO)
                }
            },
            enabled = !writing,
            fill = if (hearing) p.ink else Color.Transparent,
            modifier = Modifier.alpha(if (writing) 0.4f else 1f),
        ) {
            Glyph(Glyph.dictate, 18.dp, if (hearing) p.paper else p.mid, label = if (hearing) "Stop dictating" else if (writing) "Writing it down" else "Dictate")
        }
    }
}

/** A failure, or the words heard so far, above the composer. A popup, since the composer's card clips
 * what it holds; it takes no focus, so the keyboard stays up. */
@Composable
private fun Failed(said: String, top: Int, live: Boolean = false) {
    val p = Ink
    val gap = with(LocalDensity.current) { 6.dp.roundToPx() }
    Popup(popupPositionProvider = remember(top, gap) { OverComposer(top - gap) }) {
        Text(
            said, style = Type.small, color = if (live) p.ink else p.mid,
            modifier = Modifier.shadow(6.dp, CircleShape, ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
                .clip(CircleShape).background(p.lift).border(1.dp, p.frame, CircleShape)
                .padding(horizontal = 12.dp, vertical = 6.dp),
        )
    }
}

private class OverComposer(private val foot: Int) : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize) =
        IntOffset((anchorBounds.right - popupContentSize.width).coerceAtLeast(0), foot - popupContentSize.height)
}
