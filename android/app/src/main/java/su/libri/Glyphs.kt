package su.libri

import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** A box in SVG path words, as `<rect x y width height rx>` draws it. */
private fun rect(x: Float, y: Float, w: Float, h: Float, r: Float) =
    "M${x + r} ${y}h${w - 2 * r}a$r $r 0 0 1 $r ${r}v${h - 2 * r}a$r $r 0 0 1 -$r ${r}h${-(w - 2 * r)}a$r $r 0 0 1 -$r -${r}v${-(h - 2 * r)}a$r $r 0 0 1 $r -${r}z"

private fun circle(cx: Float, cy: Float, r: Float) = "M${cx - r} ${cy}a$r $r 0 1 0 ${2 * r} 0a$r $r 0 1 0 -${2 * r} 0z"

/** A line drawing as the web app's icons are: round-capped strokes on a
 * 24 (or 16) unit square, in the text's colour. */
private fun lines(name: String, vararg d: String, box: Float = 24f, width: Float = 2f, dots: List<String> = emptyList()) =
    ImageVector.Builder(name, 24.dp, 24.dp, box, box).apply {
        d.forEach {
            addPath(
                PathParser().parsePathString(it).toNodes(),
                stroke = SolidColor(Color.Black),
                strokeLineWidth = width,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        dots.forEach { addPath(PathParser().parsePathString(it).toNodes(), fill = SolidColor(Color.Black)) }
    }.build()

/** The web app's icons (App.tsx, work.tsx, terminal.tsx), path for path. */
object Glyph {
    val panel = lines("panel", rect(3f, 4f, 18f, 16f, 2f), "M9 4v16")
    val terminal = lines("terminal", "m5 8 4 4-4 4M12 16h7")
    val plus = lines("plus", "M12 5v14M5 12h14")
    val up = lines("up", "M12 19V5M5 12l7-7 7 7")
    val down = lines("down", "M12 5v14M19 12l-7 7-7-7")
    val chevronDown = lines("chevron-down", "M6 9l6 6 6-6")
    val chevronUp = lines("chevron-up", "M6 15l6-6 6 6")
    val chevronRight = lines("chevron-right", "M9 6l6 6-6 6")
    val chevronLeft = lines("chevron-left", "M15 6l-6 6 6 6")
    val close = lines("close", "M18 6 6 18M6 6l12 12")
    val check = lines("check", "M20 6 9 17l-5-5")
    val shield = lines("shield", "M8 1.8l5 1.9v4c0 3.1-2.1 5.5-5 6.5-2.9-1-5-3.4-5-6.5v-4z", box = 16f, width = 1.4f)
    /** A mode that asks nothing: the shield filled. */
    val shieldFull = lines("shield-full", "M8 1.8l5 1.9v4c0 3.1-2.1 5.5-5 6.5-2.9-1-5-3.4-5-6.5v-4z", box = 16f, width = 1.4f, dots = listOf("M8 1.8l5 1.9v4c0 3.1-2.1 5.5-5 6.5-2.9-1-5-3.4-5-6.5v-4z"))
    val sandbox = lines("sandbox", "M2.5 5 8 2l5.5 3v6L8 14l-5.5-3zM2.5 5 8 8l5.5-3M8 8v6", box = 16f, width = 1.4f)
    /** Nothing holds the harness in: the box filled, as the shield. */
    val sandboxOpen = lines("sandbox-open", "M2.5 5 8 2l5.5 3v6L8 14l-5.5-3zM2.5 5 8 8l5.5-3M8 8v6", box = 16f, width = 1.4f, dots = listOf("M2.5 5 8 2l5.5 3v6L8 14l-5.5-3zM2.5 5 8 8l5.5-3M8 8v6"))
    val calendar = lines("calendar", rect(3.5f, 5f, 17f, 15f, 2f), "M3.5 10h17M8 3v4M16 3v4")
    val hosts = lines("hosts", rect(3f, 4f, 18f, 7f, 2f), rect(3f, 13f, 18f, 7f, 2f), "M7 7.5h.01M7 16.5h.01")
    val more = lines("more", "M5 12h.01M12 12h.01M19 12h.01", width = 3f)
    val chat = lines("chat", "M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z")
    val work = lines("work", rect(3.5f, 4f, 17f, 6f, 2f), rect(3.5f, 14f, 17f, 6f, 2f))
    val pin = lines(
        "pin",
        "M12 17v5",
        "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z",
    )
    val copy = lines("copy", rect(9f, 9f, 12f, 12f, 2f), "M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1")
    val select = lines("select", "M4 7V4h3M20 7V4h-3M4 17v3h3M20 17v3h-3", "M9 9h6M9 12h6M9 15h4")
    val edit = lines("edit", "M12 20h9", "M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z")
    val speak = lines("speak", "M11 5 6 9H2v6h4l5 4V5z", "M15.54 8.46a5 5 0 0 1 0 7.07")
    val redraw = lines("redraw", "M21 2v6h-6M3 22v-6h6", "M3.5 9a9 9 0 0 1 14.85-3.36L21 8M21 15a9 9 0 0 1-14.85 3.36L3 16")
    val reply = lines("reply", "m9 14-5-5 5-5", "M4 9h10.5a5.5 5.5 0 0 1 0 11H11")
    val promote = lines("promote", "M14 4h6v6M20 4l-9 9", "M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4")
    val archive = lines("archive", rect(3f, 4f, 18f, 4f, 1f), "M5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4")
    // Merge into main, git's merge mark (App.tsx AgentPage).
    val merge = lines("merge", circle(18f, 18f, 3f), circle(6f, 6f, 3f), "M6 21V9a9 9 0 0 0 9 9")
    val restore = lines("restore", "M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5")
    // Temp's Clear, an eraser so it never reads as Restore's arrow.
    val eraser = lines("eraser", "M7 21l-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21M22 21H7M5 11l9 9")
    val moon = lines("moon", "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z")
    val sun = lines("sun", circle(12f, 12f, 4f), "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4")
    val clock = lines("clock", circle(12f, 12f, 9f), "M12 7v5l3 2")
    val camera = lines("camera", "M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3zM12 17a4 4 0 1 0 0-8 4 4 0 0 0 0 8z")
    val photos = lines("photos", "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21")
    val files = lines("files", "m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48")
    val gear = lines(
        "gear",
        "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
        circle(12f, 12f, 3f),
    )
    val update = lines("update", circle(12f, 12f, 9f), "M12 16V8M8 12l4-4 4 4")
    val file = lines("file", "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z", "M14 2v6h6")
    // The tool rows' glyphs (App.tsx GLYPHS).
    val brain = lines(
        "brain",
        "M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z",
        "M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z",
        "M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4M17.6 6.5a3 3 0 0 0 .4-1.5M6 5a3 3 0 0 0 .4 1.5",
    )
    val shell = lines("shell", "m4 17 6-6-6-6M12 19h8")
    val pen = lines("pen", "M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z")
    val find = lines("find", circle(11f, 11f, 7f), "m21 21-4.3-4.3")
    val web = lines("web", circle(12f, 12f, 10f), "M2 12h20M12 2a15.3 15.3 0 0 1 0 20 15.3 15.3 0 0 1 0-20")
    val agent = lines("agent", "m12 3 1.9 5.8L20 10.5l-6.1 1.7L12 18l-1.9-5.8L4 10.5l6.1-1.7z")
    val system = lines("system", "M3.5 13h4l2.5-6 4 11 2.5-5h4")
    val browser = lines("browser", circle(12f, 12f, 8.5f), "M3.5 12h17M12 3.5c2.4 2.3 3.5 5.2 3.5 8.5s-1.1 6.2-3.5 8.5M12 3.5C9.6 5.8 8.5 8.7 8.5 12s1.1 6.2 3.5 8.5")
    val board = lines("board", rect(3.5f, 4.5f, 17f, 13f, 2f), "m7 13.5 3-3 2.5 2.5L16 9.5M9 21l3-3.5 3 3.5")
    val library = lines("library", "M5 4h3v16H5zM10 4h3v16h-3z", "m15.5 5.2 2.9-.8 3.6 15-2.9.8z")
    val page = lines("page", "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z", "M14 3v5h5M9 13h6M9 17h4")
    val back = lines("back", "m15 18-6-6 6-6")
    val trash = lines("trash", "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3")
    val list = lines("list", "M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01")
    val ask = lines("ask", circle(12f, 12f, 10f), "M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3M12 17h.01")
    val plug = lines("plug", "M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z")
    /** The voice button's mark: bars like a voice's level. */
    val bars = lines("bars", "M4.5 10v4M9.5 5v14M14.5 7.5v9M19.5 10v4", width = 2.2f)
    val mic = lines("mic", rect(9f, 3f, 6f, 11f, 3f), "M5 11a7 7 0 0 0 14 0M12 18v3", width = 1.8f)
    val micOff = lines("mic-off", rect(9f, 3f, 6f, 11f, 3f), "M5 11a7 7 0 0 0 14 0M12 18v3", "M4 4l16 16", width = 1.8f)
    val dictate = lines("dictate", rect(9f, 3f, 6f, 11f, 3f), "M5 11a7 7 0 0 0 14 0M12 18v3")
    val hangUp = lines("hang-up", dots = listOf("M12 9c-2.4 0-4.6.5-6.6 1.4-.6.3-.9.9-.9 1.5v2.4c0 .7.6 1.2 1.3 1.1l2.6-.5c.6-.1 1-.6 1-1.2v-1.6c1.7-.5 3.5-.5 5.2 0v1.6c0 .6.4 1.1 1 1.2l2.6.5c.7.1 1.3-.4 1.3-1.1v-2.4c0-.6-.3-1.2-.9-1.5C16.6 9.5 14.4 9 12 9z"))
    val play = lines("play", "m7 4 12 8-12 8z")
    val stop = lines("stop", rect(6f, 6f, 12f, 12f, 1.5f))
    val thought = lines("thought", circle(8f, 8f, 6f), "M6 8h4", box = 16f, width = 1.4f)
}

/** An icon in `tint`, by default the text colour where it stands. */
@Composable
fun Glyph(v: ImageVector, size: Dp = 18.dp, tint: Color = Color.Unspecified, modifier: Modifier = Modifier, label: String? = null) =
    Icon(v, contentDescription = label, tint = if (tint == Color.Unspecified) androidx.compose.material3.LocalContentColor.current else tint, modifier = modifier.size(size))

/** A model maker's mark (logos/), as the model menu shows it. */
val MAKERS = listOf(
    Regex("^(claude|opus|sonnet|haiku|fable)") to R.drawable.logo_claude,
    Regex("^(gpt|o\\d|codex)") to R.drawable.logo_openai,
    Regex("^deepseek") to R.drawable.logo_deepseek,
    Regex("^(glm|zhipu)") to R.drawable.logo_zhipu,
    Regex("^kimi") to R.drawable.logo_moonshot,
    Regex("^grok") to R.drawable.logo_xai,
    Regex("^qwen") to R.drawable.logo_qwen,
    Regex("^gemini") to R.drawable.logo_gemini,
    Regex("^(mistral|devstral|codestral)") to R.drawable.logo_mistral,
    Regex("^minimax") to R.drawable.logo_minimax,
    Regex("^(muse|llama)") to R.drawable.logo_meta,
    Regex("^mimo") to R.drawable.logo_xiaomimimo,
    Regex("^nemotron") to R.drawable.logo_nvidia,
    Regex("^hy-|^hunyuan") to R.drawable.logo_hunyuan,
    Regex("^step") to R.drawable.logo_stepfun,
    Regex("^longcat") to R.drawable.logo_longcat,
)

fun logoOf(model: String, provider: String, harness: String): Int? {
    val id = model.substringAfterLast('/').lowercase()
    MAKERS.firstOrNull { it.first.containsMatchIn(id) }?.let { return it.second }
    if (provider.startsWith("opencode")) return R.drawable.logo_opencode
    return when (harness) {
        "claude" -> R.drawable.logo_claude
        "codex" -> R.drawable.logo_openai
        "muse" -> R.drawable.logo_meta
        else -> null
    }
}

/** Each maker's brand colour, light then dark: the brand's own where it
 * stands 3:1 against the circle's fill, else the nearest shade that does.
 * Black or white marks (OpenAI, xAI, Moonshot, OpenCode, StepFun, LongCat)
 * have none and take the text's colour. The web keeps the same table
 * (marks.tsx BRANDS). */
private val BRANDS = mapOf(
    R.drawable.logo_claude to (Color(0xFFC6532D) to Color(0xFFD97757)),
    R.drawable.logo_deepseek to (Color(0xFF4867FE) to Color(0xFF5773FE)),
    R.drawable.logo_gemini to (Color(0xFF086EFF) to Color(0xFF3186FF)),
    R.drawable.logo_hunyuan to (Color(0xFF0055E9) to Color(0xFF2776FF)),
    R.drawable.logo_meta to (Color(0xFF0074E1) to Color(0xFF0081FB)),
    R.drawable.logo_minimax to (Color(0xFFE91034) to Color(0xFFF23F5D)),
    R.drawable.logo_mistral to (Color(0xFFD74004) to Color(0xFFFA520F)),
    R.drawable.logo_nvidia to (Color(0xFF528100) to Color(0xFF76B900)),
    R.drawable.logo_qwen to (Color(0xFF615CED) to Color(0xFF736EEF)),
    R.drawable.logo_zhipu to (Color(0xFF3859FF) to Color(0xFF5773FF)),
    R.drawable.logo_xiaomimimo to (Color(0xFFC75200) to Color(0xFFFF6900)),
)

/** A mark in its maker's colour, or the text's own where the brand is
 * black or white. */
@Composable
fun Logo(res: Int?, size: Dp = 16.dp) {
    if (res == null) {
        androidx.compose.foundation.layout.Spacer(Modifier.size(size))
        return
    }
    val brand = BRANDS[res]?.let { if (LocalPaper.current.dark) it.second else it.first }
    Icon(painterResource(res), contentDescription = null, tint = brand ?: androidx.compose.material3.LocalContentColor.current, modifier = Modifier.size(size))
}
