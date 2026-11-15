package su.libri

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import kotlin.math.sqrt
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import su.libri.core.CallLine
import su.libri.core.CallPass
import su.libri.core.ClientException
import su.libri.core.Host

private const val RATE = 24_000
private const val FRAME = RATE / 50 * 2
private const val VOICE_URL = "wss://api.x.ai/v1/realtime?model=grok-voice-latest"

private enum class Line { CONNECTING, LISTENING, THINKING, SPEAKING, ENDED, FAILED }

/** A call as the screen drives it, whichever way it runs. */
private interface Calling {
    var muted: Boolean
    fun start(): Thread
    fun finish(why: String? = null)
}

/** The microphone for a call with the chat: 16 kHz mono, 20 ms frames. */
private const val EAR_RATE = 16_000
private const val EAR_FRAME = EAR_RATE / 50 * 2

/**
 * A call with the chat itself, as call.tsx's ChatCall: the microphone's
 * frames go to the host, which finds each utterance, writes it down and
 * hands it to the main chat as the person's message marked "(call)"; the
 * chat's reply comes back a sentence at a time, its words then its audio,
 * and plays in order. When the person talks over it, it stops and their
 * words steer the chat. The chat holds every word as it goes.
 */
private class ChatCall(
    private val context: Context,
    private val host: Host,
    private val speed: Float,
    private val changed: (Line, String?) -> Unit,
    private val heard: (Float) -> Unit,
    private val caption: (String) -> Unit,
    private val ended: () -> Unit,
) : Calling {
    @Volatile override var muted = false
    private val over = AtomicBoolean(false)
    private val client = OkHttpClient()
    private var socket: WebSocket? = null
    @Volatile private var opened = false
    private var record: AudioRecord? = null
    private val audio = context.getSystemService(AudioManager::class.java)
    private var modeBefore = audio.mode
    // Sentences to play, in order, each its words and its audio file.
    private val queue = LinkedBlockingQueue<Pair<String, ByteArray>>()
    @Volatile private var pendingText: String = ""
    @Volatile private var player: android.media.MediaPlayer? = null
    @Volatile private var turn = 0

    override fun start() = thread(name = "libri-chat-call") {
        try {
            openAudio()
            listen()
            thread(name = "libri-chat-call-out") { speak() }
            capture()
        } catch (e: IllegalStateException) {
            finish("The microphone is not available.")
        }
    }

    @SuppressLint("MissingPermission") // CallScreen asks before it starts.
    private fun openAudio() {
        modeBefore = audio.mode
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        if (Build.VERSION.SDK_INT >= 31) {
            audio.availableCommunicationDevices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }?.let { audio.setCommunicationDevice(it) }
        } else {
            @Suppress("DEPRECATION")
            audio.isSpeakerphoneOn = true
        }
        val least = AudioRecord.getMinBufferSize(EAR_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        record = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION, EAR_RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(least, EAR_FRAME * 4),
        ).also { check(it.state == AudioRecord.STATE_INITIALIZED) }
    }

    private fun capture() {
        val mic = record ?: return
        mic.startRecording()
        val buf = ByteArray(EAR_FRAME)
        while (!over.get()) {
            val n = mic.read(buf, 0, EAR_FRAME)
            if (n <= 0) continue
            if (muted) {
                heard(0f)
                continue
            }
            heard(level(buf, n))
            if (opened) socket?.send(buf.copyOf(n).toByteString())
        }
    }

    /** Plays each sentence as it comes, one after another. */
    private fun speak() {
        while (!over.get()) {
            val (text, file) = queue.poll(100, java.util.concurrent.TimeUnit.MILLISECONDS) ?: continue
            val mine = turn
            val done = java.util.concurrent.CountDownLatch(1)
            val p = android.media.MediaPlayer()
            runCatching {
                p.setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
                p.setDataSource(BytesSource(file))
                p.setOnCompletionListener { done.countDown() }
                p.setOnErrorListener { _, _, _ -> done.countDown(); true }
                p.prepare()
                if (speed != 1f) p.playbackParams = p.playbackParams.setSpeed(speed)
                if (mine != turn || over.get()) return@runCatching
                player = p
                caption(text)
                changed(Line.SPEAKING, null)
                p.start()
                done.await()
            }
            player = null
            p.release()
            if (queue.isEmpty() && !over.get()) changed(Line.LISTENING, null)
        }
    }

    private fun level(pcm: ByteArray, n: Int): Float {
        var sum = 0f
        var i = 0
        while (i + 1 < n) {
            val s = (pcm[i].toInt() and 0xff or (pcm[i + 1].toInt() shl 8)).toShort() / 32768f
            sum += s * s
            i += 2
        }
        return minOf(1f, sqrt(sum / (n / 2)) * 6)
    }

    private fun listen() {
        socket = client.newWebSocket(Request.Builder().url(host.callSocketUrl()).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                opened = true
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                queue.put(pendingText to bytes.toByteArray())
                pendingText = ""
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val e = runCatching { JSONObject(text) }.getOrNull() ?: return
                when (e.optString("type")) {
                    "ready" -> changed(Line.LISTENING, null)
                    "speech" -> {
                        // The person talks over the voice: it stops, and
                        // their words steer the chat.
                        if (player != null || queue.isNotEmpty()) {
                            silence()
                            webSocket.send(JSONObject().put("type", "barge").toString())
                        }
                        changed(Line.LISTENING, null)
                    }
                    "partial" -> caption(e.optString("text"))
                    "heard" -> {
                        caption(e.optString("text"))
                        changed(Line.THINKING, null)
                    }
                    "say" -> pendingText = e.optString("text")
                    "error" -> caption(e.optString("text"))
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                finish(if (code == 1000) null else "The call dropped ($code${if (reason.isEmpty()) "" else ": $reason"}).")
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                finish("The call dropped (${t.message ?: t.javaClass.simpleName}).")
            }
        })
    }

    private fun silence() {
        turn++
        queue.clear()
        runCatching { player?.stop() }
    }

    override fun finish(why: String?) {
        if (!over.compareAndSet(false, true)) return
        silence()
        socket?.close(1000, null)
        runCatching { record?.stop() }
        record?.release()
        audio.mode = modeBefore
        if (Build.VERSION.SDK_INT >= 31) {
            audio.clearCommunicationDevice()
        } else {
            @Suppress("DEPRECATION")
            audio.isSpeakerphoneOn = false
        }
        changed(if (why != null) Line.FAILED else Line.ENDED, why)
        if (why == null) thread { ended() }
    }
}

/** A sentence's audio, played from memory. */
private class BytesSource(private val data: ByteArray) : android.media.MediaDataSource() {
    override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (position >= data.size) return -1
        val n = minOf(size, data.size - position.toInt())
        System.arraycopy(data, position.toInt(), buffer, offset, n)
        return n
    }

    override fun getSize(): Long = data.size.toLong()

    override fun close() {}
}

/**
 * A call with Grok Voice in front, as call.tsx's GrokCall: the app talks
 * with the voice service directly on a short-lived pass from the host, so
 * no audio passes through the host; the microphone's 20 ms frames go up and
 * the voice's come down as 16-bit PCM. Anything that needs knowing or doing
 * the voice hands to the chat (`ask_chat`, the host's /api/call/ask) and
 * says its answer. When it ends, what was said goes to the host and into
 * the memory. `ended` runs on some thread once that is done.
 */
private class GrokCall(
    private val context: Context,
    private val host: Host,
    private val chosen: String?,
    private val speed: Float,
    private val changed: (Line, String?) -> Unit,
    private val heard: (Float) -> Unit,
    private val caption: (String) -> Unit,
    private val ended: () -> Unit,
) : Calling {
    @Volatile override var muted = false
    private val over = AtomicBoolean(false)
    private val began = System.currentTimeMillis()
    private val lines = LinkedHashMap<String, CallLine>()
    // The turns that went to the chat with ask_chat, left out of the saved
    // call since the chat already holds the request and its answer, as
    // call.tsx's GrokCall does: the person's words that asked, and Grok's
    // responses around the handing over, by item and by response. Guarded
    // by `lines`.
    private val asked = HashSet<String>()
    private val handed = HashSet<String>()
    private val responseOf = HashMap<String, String>()
    private var lastHeard: String? = null
    private var answers = 0
    private val client = OkHttpClient()
    private var socket: WebSocket? = null
    private val early = ArrayDeque<ByteArray>()
    private val down = LinkedBlockingQueue<ByteArray>()
    @Volatile private var speakingUntil = 0L
    @Volatile private var opened = false
    private var record: AudioRecord? = null
    private var track: AudioTrack? = null
    private val audio = context.getSystemService(AudioManager::class.java)
    private var modeBefore = audio.mode

    private fun line(id: String, kind: String): CallLine = synchronized(lines) {
        lines.getOrPut(id) { CallLine(kind, "") }
    }

    private fun set(id: String, kind: String, text: String) = synchronized(lines) {
        lines[id] = line(id, kind).copy(text = text)
    }

    override fun start() = thread(name = "libri-call") {
        try {
            val pass = host.callPass()
            if (over.get()) return@thread
            openAudio()
            listen(pass)
            capture()
        } catch (e: ClientException) {
            finish(e.message ?: "The call failed to start.")
        } catch (e: IllegalStateException) {
            finish("The microphone is not available.")
        }
    }

    @SuppressLint("MissingPermission") // CallScreen asks before it starts.
    private fun openAudio() {
        modeBefore = audio.mode
        audio.mode = AudioManager.MODE_IN_COMMUNICATION
        // Held out like a speakerphone, as the page's call is.
        if (Build.VERSION.SDK_INT >= 31) {
            audio.availableCommunicationDevices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }?.let { audio.setCommunicationDevice(it) }
        } else {
            @Suppress("DEPRECATION")
            audio.isSpeakerphoneOn = true
        }
        val format = AudioFormat.Builder().setSampleRate(RATE).setEncoding(AudioFormat.ENCODING_PCM_16BIT)
        val least = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        // The voice-call source brings the phone's echo cancelling and noise suppression.
        record = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(least, FRAME * 4),
        ).also { check(it.state == AudioRecord.STATE_INITIALIZED) }
        track = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
            .setAudioFormat(format.setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setBufferSizeInBytes(RATE)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()
            .also { it.play() }
        // A writer of its own, so a full buffer never holds up the socket:
        // the word that the person has started speaking must get through.
        thread(name = "libri-call-out") {
            while (!over.get()) {
                val pcm = down.poll(100, java.util.concurrent.TimeUnit.MILLISECONDS)
                if (pcm != null) {
                    track?.write(pcm, 0, pcm.size)
                } else if (speakingUntil != 0L && System.currentTimeMillis() > speakingUntil) {
                    speakingUntil = 0
                    if (!over.get()) changed(Line.LISTENING, null)
                }
            }
        }
    }

    private fun capture() {
        val mic = record ?: return
        mic.startRecording()
        val buf = ByteArray(FRAME)
        while (!over.get()) {
            val n = mic.read(buf, 0, FRAME)
            if (n <= 0) continue
            if (muted) {
                heard(0f)
                continue
            }
            heard(level(buf, n))
            val frame = buf.copyOf(n)
            synchronized(early) {
                if (opened) socket?.send(frame.toByteString()) else if (early.size < 100) early.addLast(frame)
            }
        }
    }

    private fun level(pcm: ByteArray, n: Int): Float {
        var sum = 0f
        var i = 0
        while (i + 1 < n) {
            val s = (pcm[i].toInt() and 0xff or (pcm[i + 1].toInt() shl 8)).toShort() / 32768f
            sum += s * s
            i += 2
        }
        return minOf(1f, sqrt(sum / (n / 2)) * 6)
    }

    private fun listen(pass: CallPass) {
        val ask = Request.Builder().url(pass.url ?: VOICE_URL).header("Sec-WebSocket-Protocol", "xai-client-secret.${pass.secret}").build()
        socket = client.newWebSocket(ask, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                val audio = { JSONObject().put("format", JSONObject().put("type", "audio/pcm").put("rate", RATE)).put("transport", "binary") }
                webSocket.send(
                    JSONObject().put("type", "session.update").put(
                        "session",
                        JSONObject()
                            .put("voice", chosen ?: pass.voice)
                            .put("instructions", pass.instructions)
                            .put("turn_detection", JSONObject().put("type", "server_vad"))
                            .put(
                                "tools",
                                org.json.JSONArray().put(
                                    JSONObject()
                                        .put("type", "function")
                                        .put("name", "ask_chat")
                                        .put("description", "Hand a request to the chat, which has the memory and tools, and get its answer to say.")
                                        .put(
                                            "parameters",
                                            JSONObject().put("type", "object")
                                                .put("properties", JSONObject().put("request", JSONObject().put("type", "string").put("description", "What the person wants, in their words.")))
                                                .put("required", org.json.JSONArray().put("request")),
                                        ),
                                ),
                            )
                            .put(
                                "audio",
                                JSONObject()
                                    .put("input", audio().put("transcription", JSONObject().put("model", "grok-transcribe")))
                                    .put("output", audio().put("speed", speed.coerceIn(0.7f, 1.5f).toDouble())),
                            ),
                    ).toString(),
                )
                synchronized(early) {
                    early.forEach { webSocket.send(it.toByteString()) }
                    early.clear()
                    opened = true
                }
                changed(Line.LISTENING, null)
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                val pcm = bytes.toByteArray()
                down.put(pcm)
                val now = System.currentTimeMillis()
                speakingUntil = maxOf(speakingUntil, now) + pcm.size * 1000L / (RATE * 2)
                changed(Line.SPEAKING, null)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val e = runCatching { JSONObject(text) }.getOrNull() ?: return
                val id = e.optString("item_id")
                when (e.optString("type")) {
                    "input_audio_buffer.speech_started" -> silence()
                    "input_audio_buffer.committed" -> {
                        synchronized(lines) { lastHeard = id }
                        line(id, "user")
                    }
                    // The response that says a handed-over answer.
                    "response.created" -> synchronized(lines) {
                        if (answers > 0) {
                            answers--
                            e.optJSONObject("response")?.optString("id")?.takeIf { it.isNotEmpty() }?.let { handed.add(it) }
                        }
                    }
                    "conversation.item.input_audio_transcription.completed" -> set(id, "user", e.optString("transcript"))
                    "response.output_audio_transcript.delta" -> {
                        synchronized(lines) { responseOf[id] = e.optString("response_id") }
                        val said = line(id, "talk").text + e.optString("delta")
                        set(id, "talk", said)
                        caption(said)
                    }
                    "response.output_audio_transcript.done" -> e.optString("transcript").takeIf { it.isNotEmpty() }?.let { set(id, "talk", it) }
                    "response.function_call_arguments.done" -> if (e.optString("name") == "ask_chat") {
                        synchronized(lines) {
                            lastHeard?.let { asked.add(it) }
                            e.optString("response_id").takeIf { it.isNotEmpty() }?.let { handed.add(it) }
                        }
                        val request = runCatching { JSONObject(e.optString("arguments")).optString("request") }.getOrDefault("")
                        val callId = e.optString("call_id")
                        caption(request)
                        thread(name = "libri-call-ask") {
                            val answer = runCatching { host.callAsk(request) }.getOrNull()
                            if (over.get()) return@thread
                            val output = when {
                                answer == null -> "The chat couldn't answer that."
                                answer.isEmpty() -> "The chat is still working on it; the answer will be in the chat."
                                else -> answer
                            }
                            webSocket.send(JSONObject().put("type", "conversation.item.create").put("item", JSONObject().put("type", "function_call_output").put("call_id", callId).put("output", output)).toString())
                            synchronized(lines) { answers++ }
                            webSocket.send(JSONObject().put("type", "response.create").toString())
                        }
                    }
                    "error" -> android.util.Log.w("libri", "call: $text")
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                finish(if (code == 1000) null else "The call dropped ($code${if (reason.isEmpty()) "" else ": $reason"}).")
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                finish("The call dropped (${t.message ?: t.javaClass.simpleName}).")
            }
        })
    }

    /** The person spoke: the voice stops where it is. */
    private fun silence() {
        down.clear()
        speakingUntil = 0
        track?.run {
            pause()
            flush()
            play()
        }
        changed(Line.LISTENING, null)
    }

    /** Hangs up, or ends the call with `why`, and saves what was said. */
    override fun finish(why: String?) {
        if (!over.compareAndSet(false, true)) return
        socket?.close(1000, null)
        runCatching { record?.stop() }
        record?.release()
        track?.release()
        audio.mode = modeBefore
        if (Build.VERSION.SDK_INT >= 31) {
            audio.clearCommunicationDevice()
        } else {
            @Suppress("DEPRECATION")
            audio.isSpeakerphoneOn = false
        }
        changed(if (why != null) Line.FAILED else Line.ENDED, why)
        val said = synchronized(lines) {
            lines.filter { (id, l) -> l.text.isNotBlank() && id !in asked && responseOf[id].let { it == null || it !in handed } }.values.toList()
        }
        val minutes = maxOf(1, Math.round((System.currentTimeMillis() - began) / 60_000.0)).toULong()
        thread {
            if (said.isNotEmpty()) runCatching { host.callEnd(said, minutes) }
            if (why == null) ended()
        }
    }
}

/** The whole-screen call, as the page draws it: the name, how the line
 * stands, bars with the voice's level, the words as they are said, Mute and
 * Hang up. CallService holds the microphone while the call runs, so it goes
 * on with the screen off or the app behind another. */
@Composable
fun CallScreen(model: ChatModel, close: () -> Unit) {
    val p = Ink
    val context = LocalContext.current
    val host = model.host ?: return
    val name = model.me?.name ?: model.info?.assistant ?: "Libri"
    var line by remember { mutableStateOf(Line.CONNECTING) }
    var why by remember { mutableStateOf<String?>(null) }
    var said by remember { mutableStateOf("") }
    var loud by remember { mutableFloatStateOf(0f) }
    var muted by remember { mutableStateOf(false) }
    val call: Calling = remember {
        val changed = { l: Line, w: String? ->
            if (l == Line.FAILED) why = w
            if (l == Line.FAILED || l == Line.ENDED) CallService.stop(context)
            line = l
        }
        // What was said is in the history now: read it afresh.
        val ended = {
            model.reconnect()
            close()
        }
        if (model.callMode == "grok") {
            GrokCall(context, host, model.voice, model.voiceSpeed, changed, heard = { loud = it }, caption = { said = it }, ended = ended)
        } else {
            ChatCall(context, host, model.voiceSpeed, changed, heard = { loud = it }, caption = { said = it }, ended = ended)
        }
    }
    val begin = {
        CallService.start(context)
        call.start()
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { yes ->
        if (yes) begin() else call.finish("The microphone is blocked for this app.")
    }
    LaunchedEffect(Unit) {
        // A reading would talk over the call, with its Stop hidden under it.
        hush(model)
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            begin()
        } else {
            ask.launch(Manifest.permission.RECORD_AUDIO)
        }
    }
    DisposableEffect(Unit) {
        onDispose {
            call.finish()
            CallService.stop(context)
        }
    }
    val failed = line == Line.FAILED
    BackHandler { if (failed) close() else call.finish() }
    Column(
        Modifier.fillMaxSize().background(p.paper).statusBarsPadding().navigationBarsPadding().padding(start = 24.dp, end = 24.dp, bottom = 40.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(140.dp))
        Text(name, style = type(TypeScale.heading, 1.25f, family = Faces.serif), color = p.ink)
        Text(
            when (line) {
                Line.CONNECTING -> "Connecting…"
                Line.LISTENING -> if (muted) "Muted" else "Listening"
                Line.THINKING -> "Thinking…"
                Line.SPEAKING -> "Speaking"
                Line.ENDED -> "Saving the call…"
                Line.FAILED -> why.orEmpty()
            },
            style = Type.small, color = if (failed) p.accent else p.mid, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 6.dp),
        )
        Spacer(Modifier.height(72.dp))
        CallBars(line == Line.SPEAKING, loud)
        Spacer(Modifier.height(48.dp))
        Text(said, style = type(TypeScale.prose, 1.625f), color = p.mid, textAlign = TextAlign.Center, maxLines = 4, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 300.dp))
        Spacer(Modifier.weight(1f))
        if (failed) {
            Pressable("Close", onClick = close)
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(40.dp)) {
                Round(
                    onClick = {
                        muted = !muted
                        call.muted = muted
                    },
                    fill = if (muted) p.ink else Color.Transparent, edge = p.frame, size = 64.dp,
                ) { Glyph(if (muted) Glyph.micOff else Glyph.mic, 24.dp, if (muted) p.paper else p.ink, label = if (muted) "Unmute" else "Mute") }
                Box(
                    Modifier.size(64.dp).clip(CircleShape).background(Color(0xFFC4453A)).clickable(enabled = line != Line.ENDED) { call.finish() },
                    contentAlignment = Alignment.Center,
                ) { Glyph(Glyph.hangUp, 28.dp, Color.White, label = "Hang up") }
            }
        }
    }
}

/** Five bars: the person's level while they speak, a wave while the voice does. */
@Composable
private fun CallBars(speaking: Boolean, loud: Float) {
    val p = Ink
    val wave = rememberInfiniteTransition(label = "call")
    Row(Modifier.height(96.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(0.55f, 0.85f, 1f, 0.85f, 0.55f).forEachIndexed { i, w ->
            val talk by wave.animateFloat(
                initialValue = 0.2f,
                targetValue = 1f,
                animationSpec = infiniteRepeatable(tween(450, delayMillis = i * 120), RepeatMode.Reverse),
                label = "bar$i",
            )
            Box(Modifier.width(10.dp).height((12 + w * (if (speaking) talk else loud) * 84).dp).background(p.ink, CircleShape))
        }
    }
}
