package su.libri

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import su.libri.core.Offer
import su.libri.core.WorkChoice

/** The Tasks +: a whole screen to write a new worker's brief in, as Claude's
 * Tasks sheet is, rather than a line over the list. A close at the top left,
 * a heading and one line of help, then the main chat's own ComposerBox and
 * Pickers sitting on the keyboard: files, model, thinking, access and the mic
 * all come as they do there. The pickers change only what this worker will
 * run on, held here until Start; the Workers default is where they begin. */
@Composable
fun TaskSheet(model: ChatModel, close: () -> Unit) {
    val p = Ink
    val draft = model.taskDraft
    var choice by remember { mutableStateOf<WorkChoice?>(null) }
    // Sent only once changed, so an untouched sheet follows the Workers
    // default as a worker started anywhere else does.
    var asked by remember { mutableStateOf<WorkChoice?>(null) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(model.host) {
        model.host?.let { h -> runCatching { withContext(Dispatchers.IO) { h.workDefaultChoice() } }.getOrNull() }?.let { asked = it; choice = it }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
    // Another harness has other modes: its own defaults stand until picked.
    val choose = { o: Offer?, effort: String?, access: String?, sandbox: String? ->
        choice = choice?.let { c ->
            val moved = o != null && o.harness != c.harness
            c.copy(
                harness = o?.harness ?: c.harness,
                model = o?.model ?: c.model,
                effort = effort ?: c.effort,
                access = access ?: if (moved) "" else c.access,
                sandbox = sandbox ?: if (moved) "" else c.sandbox,
            )
        }
    }
    val start = {
        model.startWorker(draft.text.trim(), draft.files.toList(), choice?.takeIf { it != asked })
        draft.text = ""
        draft.files.clear()
        close()
    }
    BackHandler(onBack = close)
    BoxWithConstraints(
        Modifier.fillMaxSize().background(p.paper)
            .windowInsetsPadding(WindowInsets.statusBars.union(WindowInsets.navigationBars).union(WindowInsets.ime)),
    ) {
        val room = maxHeight
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Round(onClick = close, size = 40.dp) { Glyph(Glyph.close, 20.dp, p.mid, label = "Close") }
            }
            // The words sit on the keyboard, the heading just above them, as the
            // chat's composer sits at its foot.
            Spacer(Modifier.weight(1f))
            Column(Modifier.padding(start = 22.dp, end = 22.dp, bottom = 12.dp)) {
                Text("New task", style = type(TypeScale.prose, 1.3f, FontWeight.SemiBold), color = p.ink)
                Text(
                    "A worker takes it from here and reports to its card in Tasks.",
                    style = Type.note, color = p.mid, modifier = Modifier.padding(top = 4.dp),
                )
            }
            Column(Modifier.padding(start = 14.dp, end = 14.dp, bottom = 8.dp)) {
                ComposerBox(
                    model, draft, "What should a new worker do?", room * 0.4f,
                    account = choice?.let { model.account(it.harness, it.model) },
                    focus = focus,
                    pickers = { at ->
                        val own = choice
                        if (own != null) Pickers(model, at, worker = own, choose = choose) else Spacer(at)
                    },
                ) { empty -> SendButton(enabled = !empty, onClick = start) }
            }
        }
    }
}
