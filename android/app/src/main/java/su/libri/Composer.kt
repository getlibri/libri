package su.libri

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.flow.first
import androidx.compose.ui.Alignment
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import coil3.compose.AsyncImage
import su.libri.core.Ask
import su.libri.core.Card
import su.libri.core.Account
import su.libri.core.Models
import su.libri.core.Offer
import su.libri.core.Quote
import su.libri.core.WorkChoice
import su.libri.core.commands

/** The words being written and the files going with them, kept above the
 * composer so a message's Edit can fill it. */
class Draft {
    /** A TextFieldState and not a TextFieldValue: its field keeps the cursor in
     * view when the box shrinks under the keyboard, which the older field doesn't. */
    val typed = TextFieldState()

    /** The words; set from outside, the cursor goes to their end. */
    var text: String
        get() = typed.text.toString()
        set(v) = typed.setTextAndPlaceCursorAtEnd(v)
    val files = mutableStateListOf<Picked>()
    /** The earlier message the next send replies to. */
    var quote by mutableStateOf<Quote?>(null)
    /** Counts asks for the field's focus and keyboard, as a Reply or an Edit
     * makes: the words go where the person is about to type. */
    var focusAsked by mutableIntStateOf(0)
        private set
    fun focus() { focusAsked++ }
}

/** The composer as the phone page draws it, for the main chat: what the
 * box holds is `ComposerBox`. */
@Composable
fun Composer(model: ChatModel, draft: Draft, typing: Boolean, trouble: String?, textMost: Dp) {
    val p = Ink
    val send = {
        val text = draft.text
        if (text.isNotBlank() || draft.files.isNotEmpty()) model.send(text, draft.files.toList(), draft.quote)
        draft.files.clear()
        draft.quote = null
        draft.text = ""
    }
    val name = model.me?.name ?: model.info?.assistant ?: "Libri"
    Column(Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 6.dp, bottom = 8.dp)) {
        Commands(model, draft)
        draft.quote?.let { q ->
            Row(Modifier.padding(start = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                QuoteLine(q, Modifier.weight(1f))
                Quiet("Cancel") { draft.quote = null }
            }
        }
        ComposerBox(model, draft, messageTo(model.me?.app), textMost, account = model.account(model.models?.harness, model.models?.model), harness = model.models?.harness, agents = AgentScope.Main, pickers = { Pickers(model, it) }) { empty ->
            // Words sent mid-turn join it; Stop shows only with nothing to send.
            if (typing && empty) {
                StopButton(onClick = model::stopTurn)
            } else if (empty && (model.me?.call ?: (model.me?.voice != null))) {
                // An empty box offers a call, as the page's does.
                Round(onClick = { model.calling = true }, fill = p.ink) { Glyph(Glyph.bars, 20.dp, p.paper, label = "Call $name") }
            } else {
                SendButton(enabled = !empty, onClick = send)
            }
        }
        if (trouble != null) Small(trouble, Modifier.padding(start = 8.dp, top = 6.dp))
    }
}

@Composable
fun StopButton(enabled: Boolean = true, onClick: () -> Unit) {
    val p = Ink
    Round(onClick = onClick, enabled = enabled, edge = p.frame, modifier = Modifier.alpha(if (enabled) 1f else 0.5f)) {
        Box(Modifier.size(12.dp).clip(RoundedCornerShape(2.dp)).background(p.ink).semantics { contentDescription = "Stop" })
    }
}

@Composable
fun SendButton(enabled: Boolean, onClick: () -> Unit) {
    val p = Ink
    Round(onClick = onClick, enabled = enabled, fill = if (enabled) p.ink else p.hair) {
        Glyph(Glyph.up, 18.dp, if (enabled) p.paper else p.soft, label = "Send")
    }
}

/** The words of a draft, wherever one is written. A long draft shows its end:
 * the field follows the cursor only while focused, so words set from outside
 * (a kept draft, dictation, an Edit) would otherwise rest on their first line,
 * and the tap that opens the keyboard would land up there. */
@Composable
fun DraftField(draft: Draft, placeholder: String, modifier: Modifier = Modifier, lineLimits: TextFieldLineLimits = TextFieldLineLimits.Default, focus: FocusRequester? = null) {
    val p = Ink
    val scroll = rememberScrollState()
    val interaction = remember { MutableInteractionSource() }
    val focused by interaction.collectIsFocusedAsState()
    LaunchedEffect(scroll.maxValue) { if (!focused) scroll.scrollTo(scroll.maxValue) }
    // The Tasks sheet passes its own, to focus the field when it opens.
    val field = focus ?: remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val window = LocalWindowInfo.current
    LaunchedEffect(draft.focusAsked) {
        if (draft.focusAsked == 0) return@LaunchedEffect
        // The ask comes from a hold menu, a focusable popup that holds the
        // window's focus until it is gone; asked before, the field never gets it.
        snapshotFlow { window.isWindowFocused }.first { it }
        field.requestFocus()
        keyboard?.show()
    }
    BasicTextField(
        state = draft.typed,
        textStyle = Type.body.copy(color = p.ink),
        cursorBrush = SolidColor(p.ink),
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        lineLimits = lineLimits,
        interactionSource = interaction,
        scrollState = scroll,
        modifier = modifier.fillMaxWidth().focusRequester(field).padding(horizontal = 10.dp, vertical = 7.dp),
        decorator = { inner -> Box { if (draft.text.isEmpty()) Text(placeholder, style = Type.body, color = p.soft); inner() } },
    )
}

/** The input every long-lived talk shares: a --lift card with a --frame edge
 * and 24px corners, the files and words above; below them the +, `pickers`,
 * the usage rings of `account` and one `trailing` button, told whether the box is empty.
 * `dashed`: the edge that says these words are not kept. */
@Composable
fun ComposerBox(
    model: ChatModel,
    draft: Draft,
    placeholder: String,
    textMost: Dp,
    dashed: Boolean = false,
    account: Account? = null,
    focus: FocusRequester? = null,
    /** The harness the words go to, whose skills the `$` menu lists. */
    harness: String? = null,
    /** Whose agents at work show in the box's outline (Agents.kt). */
    agents: AgentScope? = null,
    pickers: @Composable (Modifier) -> Unit,
    trailing: @Composable (empty: Boolean) -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        Skills(model, draft, harness)
        // Where the outline breaks for the agents' circles, while any run.
        var width by remember { mutableIntStateOf(0) }
        Box(Modifier.fillMaxWidth().onSizeChanged { width = it.width }) {
            ComposerFrame(model, draft, placeholder, textMost, dashed, account, focus, pickers, trailing)
            // Centred on the outline at the top right, clear of its corner.
            agents?.let { ComposerAgents(model, it, width, Modifier.align(Alignment.TopEnd).padding(end = AGENTS_END).offset(y = (-11).dp)) }
        }
    }
}

@Composable
private fun ComposerFrame(
    model: ChatModel,
    draft: Draft,
    placeholder: String,
    textMost: Dp,
    dashed: Boolean,
    account: Account?,
    focus: FocusRequester?,
    pickers: @Composable (Modifier) -> Unit,
    trailing: @Composable (empty: Boolean) -> Unit,
) {
    val p = Ink
    val context = LocalContext.current
    var adding by remember { mutableStateOf(false) }
    var shot by remember { mutableStateOf<Uri?>(null) }
    var top by remember { mutableIntStateOf(0) }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        uris.forEach { draft.files += model.picked(it) }
    }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        uris.forEach { draft.files += model.picked(it) }
    }
    val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        val at = shot
        if (taken && at != null) draft.files += Picked(at, "photo-${System.currentTimeMillis()}.jpg", "image/jpeg")
    }
    Column(
        Modifier.fillMaxWidth().onGloballyPositioned { top = it.boundsInWindow().top.toInt() }
            // The one floating make (Kit.kt floating); Temp's dashed edge marks
            // a talk nothing keeps, so it is drawn over the hairline's place.
            .then(
                if (dashed) Modifier.shadow(4.dp, RoundedCornerShape(Radii.composer), ambientColor = Color(0x22000000), spotColor = Color(0x22000000))
                    .clip(RoundedCornerShape(Radii.composer)).background(p.lift).dashedEdge(p.frame, Radii.composer)
                else Modifier.floating(p, RoundedCornerShape(Radii.composer)),
            )
            .padding(start = 6.dp, end = 5.dp, top = 5.dp, bottom = 5.dp),
    ) {
        if (draft.files.isNotEmpty()) {
            FlowRow(Modifier.padding(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                draft.files.forEach { f -> Chip(f) { draft.files.remove(f) } }
            }
        }
        DraftField(draft, placeholder, Modifier.heightIn(max = textMost), focus = focus)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Box {
                Round(onClick = { adding = true }) { Glyph(Glyph.plus, 20.dp, p.mid, label = "Attach") }
                PaperMenu(adding, { adding = false }, offset = DpOffset(0.dp, (-8).dp)) {
                    MenuRow("Camera", icon = Glyph.camera, onClick = {
                        adding = false
                        val dir = context.cacheDir.resolve("camera").apply { mkdirs() }
                        val file = dir.resolve("photo-${System.currentTimeMillis()}.jpg")
                        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
                        shot = uri
                        camera.launch(uri)
                    })
                    MenuRow("Photos", icon = Glyph.photos, onClick = {
                        adding = false
                        photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                    })
                    MenuRow("Files", icon = Glyph.files, onClick = {
                        adding = false
                        documents.launch(arrayOf("*/*"))
                    })
                }
            }
            pickers(Modifier.weight(1f))
            account?.let { Rings(it) }
            if (model.dictateOnPhone || model.me?.dictate == true) Dictate(model, top) { draft.text = joined(draft.text, it) }
            trailing(draft.text.isBlank() && draft.files.isEmpty())
        }
    }
}

/** A chosen file: its photo, or its name on a card, with a way to take it off. */
@Composable
private fun Chip(f: Picked, remove: () -> Unit) {
    val p = Ink
    Box {
        if (f.image) {
            AsyncImage(
                model = f.uri,
                contentDescription = f.name,
                contentScale = ContentScale.Crop,
                modifier = Modifier.size(64.dp).clip(RoundedCornerShape(Radii.xl)).border(1.dp, p.hair, RoundedCornerShape(Radii.xl)),
            )
        } else {
            Row(
                Modifier.height(64.dp).widthIn(max = 200.dp).clip(RoundedCornerShape(Radii.xl)).border(1.dp, p.hair, RoundedCornerShape(Radii.xl))
                    .background(p.paper).padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Glyph(Glyph.file, 18.dp, p.mid)
                Text(f.name, style = Type.note, color = p.ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        Box(
            Modifier.align(Alignment.TopEnd).padding(2.dp).size(20.dp).clip(CircleShape).background(p.ink).clickable(onClick = remove),
            contentAlignment = Alignment.Center,
        ) { Glyph(Glyph.close, 12.dp, p.paper, label = "Remove") }
    }
}

/** The commands that match what is typed after a slash, as a card over the
 * composer: Libri's, then the main chat's harness's own, each tagged with
 * its harness and sent to it as typed (the page's `SlashInput`). */
@Composable
private fun Commands(model: ChatModel, draft: Draft) {
    val p = Ink
    val typed = Regex("^/([\\w:.-]*)$").find(draft.text)?.groupValues?.get(1)
    val open = typed != null
    LaunchedEffect(open) { if (open) model.refreshCommands() }
    if (typed == null) return
    val all = remember { commands() }
    val own = model.harnessCommands
    val theirs = own?.commands.orEmpty().filter { n -> all.none { it.name == n } }
    val list = all.filter { it.name.startsWith(typed.lowercase()) }
    val listed = theirs.filter { it.startsWith(typed.lowercase()) }
    if (list.isEmpty() && listed.isEmpty()) return
    Column(Modifier.padding(bottom = 6.dp).fillMaxWidth().heightIn(max = 320.dp).card(p, radius = Radii.xxl, framed = false).border(1.dp, p.frame, RoundedCornerShape(Radii.xxl)).verticalScroll(rememberScrollState()).padding(vertical = 6.dp)) {
        list.forEach { c ->
            Row(
                Modifier.fillMaxWidth().clickable { draft.text = if (c.arg.isEmpty()) "/${c.name}" else "/${c.name} " }.padding(horizontal = 16.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("/${c.name}", style = type(TypeScale.control, 1.4f, family = Faces.mono), color = p.ink)
                if (c.arg.isNotEmpty()) Text(c.arg, style = type(TypeScale.control, 1.4f, family = Faces.mono), color = p.soft)
                Text(c.does, style = Type.note, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        listed.forEach { n ->
            Row(
                // A harness's command may take words after it, which it alone knows.
                Modifier.fillMaxWidth().clickable { draft.text = "/$n " }.padding(horizontal = 16.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("/$n", style = type(TypeScale.control, 1.4f, family = Faces.mono), color = p.ink, modifier = Modifier.weight(1f, fill = false))
                Text(harnessLabel(own!!.harness), style = Type.note, color = p.soft, maxLines = 1)
            }
        }
    }
}

private fun harnessLabel(h: String) = when (h) {
    "claude" -> "Claude Code"
    "codex" -> "Codex"
    "opencode" -> "OpenCode"
    "pi" -> "Pi"
    "muse" -> "Muse"
    else -> h
}

/** The `$` skill menu, as the page's (skills.tsx `useSkillMenu`): a `$`
 * starting a word at the end of the words lists the skills the harness at
 * hand finds, Libri's and the person's marked apart; a tap puts the
 * harness's own way of naming one in place of what was typed. */
@Composable
private fun Skills(model: ChatModel, draft: Draft, harness: String?) {
    val p = Ink
    val typed = Regex("(^|\\s)\\$([a-z0-9-]*)$").find(draft.text)?.groupValues?.get(2)
    val open = typed != null
    LaunchedEffect(open) { if (open) model.refreshSkills() }
    if (typed == null) return
    val menus = model.skillMenus
    // A host with one harness may not say which the thread is on.
    val menu = menus.find { it.harness == harness } ?: menus.singleOrNull() ?: return
    val list = menu.skills.filter { it.name.startsWith(typed) }
    if (list.isEmpty()) return
    Column(
        Modifier.padding(bottom = 6.dp).fillMaxWidth().heightIn(max = 320.dp).card(p, radius = Radii.xxl, framed = false)
            .border(1.dp, p.frame, RoundedCornerShape(Radii.xxl)).verticalScroll(rememberScrollState()).padding(vertical = 6.dp),
    ) {
        list.forEach { s ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 44.dp).clickable {
                    val words = draft.text
                    draft.text = words.dropLast(typed.length + 1) + menu.mention + s.name + " "
                }.padding(horizontal = 16.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(menu.mention + s.name, style = type(TypeScale.control, 1.4f, family = Faces.mono), color = p.ink)
                Text(s.description, style = Type.small, color = p.mid, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Text(if (s.source == "libri") "Libri" else "Yours", style = Type.note, color = p.soft)
            }
        }
    }
}

private val PROVIDERS = mapOf(
    "anthropic" to "Anthropic", "openai" to "OpenAI", "google" to "Google", "xai" to "xAI",
    "deepseek" to "DeepSeek", "opencode" to "OpenCode", "mistral" to "Mistral", "moonshotai" to "Moonshot",
    "meta" to "Meta",
)

/** The model, thinking and access pills (App.tsx ModelPickers): the
 * maker's mark and name; a gauge for the thinking; a shield for access.
 * `temp` shows Temp's own. Access is the modes of the harness in use, and a
 * sandbox pill follows where that harness shows one. `worker` shows a Work thread's
 * own, and `choose` sets them: harness and model, effort, access or
 * sandbox, whichever changed. */
@Composable
fun Pickers(
    model: ChatModel,
    modifier: Modifier,
    temp: Boolean = false,
    worker: WorkChoice? = null,
    choose: (Offer?, String?, String?, String?) -> Unit = { _, _, _, _ -> },
) {
    val p = Ink
    val m: Models = model.models ?: return Spacer(modifier)
    val harness = worker?.harness ?: if (temp) m.tempHarness else m.harness
    val picked = worker?.model ?: if (temp) m.tempModel else m.model
    val effort = worker?.effort ?: if (temp) m.tempEffort else m.effort
    val access = worker?.access ?: if (temp) m.tempAccess else m.access
    val sandbox = worker?.sandbox ?: if (temp) m.tempSandbox else m.sandbox
    val modes = m.modes.find { it.harness == harness }
    // A pick stored as an alias ("opus") shows as the model it stands for.
    val now = offerOf(harness, picked, m.offers)
    var which by remember { mutableStateOf<String?>(null) }
    Row(modifier.horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
        // The model and its thinking read as one line, "Opus 5.5 High", as
        // T3 Code sets them: each word opens its own menu (App.tsx WORD).
        Box(Modifier.padding(start = 4.dp)) {
            Word(which == "model", { which = "model" }, label = "Model") {
                // No model known yet, no mark: an empty slot would only push the words off.
                now?.let { logoOf(it.model, it.provider, it.harness) }?.let { Logo(it, 15.dp) }
                Text(now?.name ?: picked.ifEmpty { "Default" }, style = type(TypeScale.label, 1.3f), maxLines = 1)
            }
            if (which == "model") ModelMenu(m, harness, picked, close = { which = null }) { if (worker != null) choose(it, null, null, null) else model.pickModel(it.harness, it.model, temp) }
        }
        val efforts = now?.efforts.orEmpty()
        if (efforts.isNotEmpty()) {
            val all = listOf("") + efforts
            fun named(e: String) = e.ifEmpty { "Default" }.replaceFirstChar { it.uppercase() }
            Box {
                Word(which == "effort", { which = "effort" }, label = "Thinking") { Text(named(if (effort in efforts) effort else ""), style = type(TypeScale.label, 1.3f), maxLines = 1) }
                PaperMenu(which == "effort", { which = null }, offset = DpOffset(0.dp, (-8).dp)) {
                    all.forEach { e ->
                        MenuRow(
                            named(e),
                            picked = e == effort,
                            onClick = {
                                which = null
                                if (worker != null) choose(null, e, null, null) else model.pickEffort(e, temp)
                            },
                        )
                    }
                }
            }
        }
        if (modes != null) {
            fun shield(open: Boolean) = if (open) Glyph.shieldFull else Glyph.shield
            Box {
                Pill(which == "access", { which = "access" }) { Glyph(shield(modes.access.any { it.id == access && it.open }), 15.dp, label = "Access") }
                PaperMenu(which == "access", { which = null }, offset = DpOffset(0.dp, (-8).dp)) {
                    // A hidden mode stays listed while it is the one in force.
                    modes.access.filter { !it.hidden || it.id == access }.forEach { o ->
                        MenuRow(
                            o.name,
                            note = o.note,
                            lead = { Glyph(shield(o.open), 16.dp, p.mid) },
                            picked = o.id == access,
                            onClick = {
                                which = null
                                if (worker != null) choose(null, null, o.id, null) else model.pickAccess(o.id, temp)
                            },
                        )
                    }
                }
            }
            fun box(open: Boolean) = if (open) Glyph.sandboxOpen else Glyph.sandbox
            // Claude Code keeps its sandbox behind /sandbox: its pill shows only
            // while one holds, since auto-allow changes what runs unasked.
            if (modes.sandbox.any { !it.hidden || (it.id == sandbox && !it.open) }) Box {
                Pill(which == "sandbox", { which = "sandbox" }) { Glyph(box(modes.sandbox.any { it.id == sandbox && it.open }), 15.dp, label = "Sandbox") }
                PaperMenu(which == "sandbox", { which = null }, offset = DpOffset(0.dp, (-8).dp)) {
                    modes.sandbox.forEach { o ->
                        MenuRow(
                            o.name,
                            note = o.note,
                            lead = { Glyph(box(o.open), 16.dp, p.mid) },
                            picked = o.id == sandbox,
                            onClick = {
                                which = null
                                if (worker != null) choose(null, null, null, o.id) else model.pickSandbox(o.id, temp)
                            },
                        )
                    }
                }
            }
        }
    }
}

/** A word of the model line: 32px tall, --mid until pressed, no chevron;
 * only the pressed fill shows where it ends. */
@Composable
private fun Word(open: Boolean, onClick: () -> Unit, label: String, content: @Composable () -> Unit) {
    val p = Ink
    Row(
        Modifier.height(32.dp).clip(RoundedCornerShape(Radii.md)).background(if (open) p.hover else Color.Transparent)
            .clickable(onClickLabel = label, onClick = onClick).padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        CompositionLocalProvider(LocalContentColor provides if (open) p.ink else p.mid) { content() }
    }
}

/** A pill: 32px tall, --mid until pressed, a small chevron after it. */
@Composable
private fun Pill(open: Boolean, onClick: () -> Unit, content: @Composable () -> Unit) {
    val p = Ink
    Row(
        Modifier.height(32.dp).clip(CircleShape).background(if (open) p.hover else Color.Transparent).clickable(onClick = onClick).padding(horizontal = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        CompositionLocalProvider(LocalContentColor provides if (open) p.ink else p.mid) {
            content()
            Glyph(Glyph.chevronDown, 12.dp)
        }
    }
}

/** The models by provider, each with its maker's mark, searchable. */
@Composable
private fun ModelMenu(m: Models, harness: String, picked: String, close: () -> Unit, pick: (Offer) -> Unit) {
    val p = Ink
    var query by remember { mutableStateOf("") }
    PaperMenu(true, close, offset = DpOffset(0.dp, (-8).dp), modifier = Modifier.width(300.dp).heightIn(max = 460.dp)) {
        BasicTextField(
            value = query,
            onValueChange = { query = it },
            textStyle = Type.small.copy(color = p.ink),
            cursorBrush = SolidColor(p.ink),
            singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
            decorationBox = { inner -> Box { if (query.isEmpty()) Text("Search models", style = Type.small, color = p.soft); inner() } },
        )
        Rule()
        val words = query.lowercase().split(' ').filter { it.isNotBlank() }
        m.offers
            .filter { o -> "${o.name} ${o.model} ${o.provider} ${PROVIDERS[o.provider].orEmpty()}".lowercase().let { t -> words.all { it in t } } }
            .groupBy { it.provider }
            .forEach { (provider, offers) ->
                Head(PROVIDERS[provider] ?: provider, Modifier.padding(start = 16.dp, top = 10.dp, bottom = 2.dp))
                offers.forEach { o ->
                    MenuRow(
                        o.name,
                        lead = { Logo(logoOf(o.model, o.provider, o.harness)) },
                        picked = o.harness == harness && o.model == picked,
                        onClick = { close(); pick(o) },
                    )
                }
            }
    }
}

/** Rings by the send button, after Paseo's context meter, for the picked
 * model's subscription (App.tsx UsageRings): its shortest window outside,
 * the next inside, one ring for one window; ink from 70%, the red pencil
 * past 90%. A tap lists every window with its reset. */
@Composable
private fun Rings(account: Account) {
    val shown = account.windows.take(2)
    if (shown.isEmpty()) return
    val p = Ink
    var open by remember { mutableStateOf(false) }
    Box {
        Box(
            Modifier.size(28.dp, 38.dp).clip(CircleShape).clickable { open = true }
                .semantics { contentDescription = "${account.name} use: " + account.windows.joinToString(", ") { "${it.name} ${it.pct.toInt()}%" } },
            contentAlignment = Alignment.Center,
        ) {
            Canvas(Modifier.size(22.dp)) {
                shown.forEachIndexed { i, l ->
                    val inset = (if (i == 0) 2 else 6).dp.toPx()
                    val pct = l.pct.toFloat().coerceIn(0f, 100f)
                    val stroke = Stroke(width = 2.dp.toPx(), cap = StrokeCap.Round)
                    val at = Offset(inset, inset)
                    val box = Size(size.width - 2 * inset, size.height - 2 * inset)
                    drawArc(p.hair, 0f, 360f, false, at, box, style = stroke)
                    val fill = if (pct > 90) p.accent else if (pct >= 70) p.ink else p.mid
                    if (pct > 0) drawArc(fill, -90f, 3.6f * pct, false, at, box, style = stroke)
                }
            }
        }
        PaperMenu(open, { open = false }, offset = DpOffset(0.dp, (-8).dp)) {
            account.windows.forEach { l ->
                MenuRow("${account.name} · ${l.name}", note = "${l.pct.toInt()}% · resets ${l.resets}", onClick = { open = false })
            }
        }
    }
}

/** A card waiting on the person, as the page draws one: --lift, framed in
 * --edge, with "WAITING ON YOU" in red pencil set into its top edge. */
@Composable
private fun Waiting(label: String, content: @Composable () -> Unit) {
    val p = Ink
    Box(Modifier.fillMaxWidth().padding(top = 8.dp)) {
        Column(Modifier.fillMaxWidth().card(p)) { content() }
        Text(
            label.uppercase(),
            style = type(TypeScale.caption, 1.2f, FontWeight.SemiBold, spacing = androidx.compose.ui.unit.TextUnit(0.04f, androidx.compose.ui.unit.TextUnitType.Em)),
            color = p.accent,
            modifier = Modifier.padding(start = 14.dp).offset(y = (-7).dp).background(p.lift, RoundedCornerShape(Radii.sm)).padding(horizontal = 6.dp),
        )
    }
}

/** What the turn asks the person, above the composer: questions to pick
 * from as chips, or whether a tool may run. Working until the host has the
 * answer; `reply` calls back `failed` if it was refused or never got there. */
@Composable
fun AskCard(a: Ask, reply: (yes: Boolean, answers: Map<String, String>, always: Boolean, failed: () -> Unit) -> Unit) {
    val p = Ink
    val picked = remember(a.id) { mutableStateMapOf<String, List<String>>() }
    val other = remember(a.id) { mutableStateMapOf<String, String>() }
    var busy by remember(a.id) { mutableStateOf(false) }
    fun answer(q: String) = (picked[q].orEmpty() + listOfNotNull(other[q]?.trim()?.takeIf { it.isNotEmpty() })).joinToString(", ")
    Waiting("Waiting on you") {
        Column(Modifier.padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (a.questions.isEmpty()) {
                Text("May I use ${a.tool}?", style = type(TypeScale.body, 1.4f, FontWeight.Medium), color = p.ink)
                a.about?.let { Text(it, style = Type.small, color = p.mid) }
                Text(
                    a.detail, style = type(TypeScale.caption, 1.45f, family = Faces.mono), color = p.ink,
                    modifier = Modifier.fillMaxWidth().heightIn(max = 160.dp).clip(RoundedCornerShape(Radii.lg)).background(p.hover)
                        .verticalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp),
                )
            }
            a.questions.forEach { q ->
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    q.header?.let { Head(it) }
                    Text(q.question, style = type(TypeScale.body, 1.4f, FontWeight.Medium), color = p.ink)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        q.options.forEach { o ->
                            val on = o.label in picked[q.question].orEmpty()
                            Column(
                                Modifier.clip(RoundedCornerShape(Radii.xl)).background(if (on) p.ink else Color.Transparent)
                                    .border(1.dp, if (on) p.ink else p.frame, RoundedCornerShape(Radii.xl))
                                    .clickable {
                                        val was = picked[q.question].orEmpty()
                                        picked[q.question] = if (q.multi) (if (on) was - o.label else was + o.label) else listOf(o.label)
                                    }.padding(horizontal = 12.dp, vertical = 6.dp),
                            ) {
                                Text(o.label, style = type(TypeScale.control, 1.35f), color = if (on) p.paper else p.ink)
                                o.description?.takeIf { it != o.label }?.let {
                                    Text(it, style = type(TypeScale.caption, 1.3f), color = if (on) p.paper.copy(alpha = 0.75f) else p.mid)
                                }
                            }
                        }
                    }
                    Field(other[q.question].orEmpty(), { other[q.question] = it }, "Other", lines = 2)
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, bottom = 12.dp), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
            if (busy) {
                Working()
            } else {
                Quiet(if (a.questions.isEmpty()) "Deny" else "Skip", p.mid) { busy = true; reply(false, emptyMap(), false) { busy = false } }
                Spacer(Modifier.width(6.dp))
                // A connection's tool: "Always allow" sets it to Allow in Settings > Connections.
                if (a.always) {
                    Pressable("Always allow") { busy = true; reply(true, emptyMap(), true) { busy = false } }
                    Spacer(Modifier.width(6.dp))
                }
                val ready = a.questions.all { answer(it.question).isNotEmpty() }
                Pressable(if (a.questions.isNotEmpty()) "Answer" else if (a.always) "Allow once" else "Allow", filled = true, enabled = ready) {
                    busy = true
                    reply(true, a.questions.associate { it.question to answer(it.question) }, false) { busy = false }
                }
            }
        }
    }
}

@Composable
private fun Working() {
    val p = Ink
    Row(Modifier.height(36.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp, color = Active, trackColor = p.hair)
        Text("Working", style = type(TypeScale.label, 1.3f), color = p.mid)
    }
}

/** A mail draft waiting for the person's word, as a letter on a sheet:
 * its heads, a dashed rule, the body in serif folded to three lines. */
@Composable
fun Letter(c: Card, decide: (Boolean) -> Unit) {
    val p = Ink
    var open by remember { mutableStateOf(false) }
    var busy by remember(c.id, c.error) { mutableStateOf(false) }
    Waiting("Draft · waiting on you") {
        Column(Modifier.padding(start = 20.dp, end = 20.dp, top = 16.dp, bottom = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            @Composable fun line(head: String, value: String, weight: FontWeight = FontWeight.Normal, color: Color = p.ink) = Row {
                Text(head, style = type(TypeScale.caption, 1.6f, FontWeight.SemiBold), color = p.soft, modifier = Modifier.width(64.dp))
                Text(value, style = type(TypeScale.control, 1.45f, weight), color = color, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            line("TO", c.to)
            c.account?.let { line("FROM", it, color = p.mid) }
            line("SUBJECT", c.subject, FontWeight.Medium)
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(p.frame.copy(alpha = 0.6f)))
        Text(
            c.body,
            style = type(TypeScale.body, 1.625f, family = Faces.serif),
            color = p.ink,
            maxLines = if (open) Int.MAX_VALUE else 3,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.fillMaxWidth().clickable { open = !open }.padding(horizontal = 20.dp, vertical = 12.dp),
        )
        c.error?.let { Text(it, style = Type.note, color = p.accent, modifier = Modifier.padding(horizontal = 20.dp)) }
        Row(Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, bottom = 12.dp), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
            if (busy) {
                Working()
            } else {
                Quiet("Drop", p.mid) { busy = true; decide(false) }
                Spacer(Modifier.width(6.dp))
                Pressable("Send it", filled = true) { busy = true; decide(true) }
            }
        }
    }
}

/** A composer's invitation: to the one it talks to by name, else plainly
 * (App.tsx messageTo). */
fun messageTo(name: String?): String = name?.takeIf { it.isNotBlank() }?.let { "Message $it" } ?: "Send a message"
