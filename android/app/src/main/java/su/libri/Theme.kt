package su.libri

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat

/**
 * The web app's design tokens (libri/site/src/index.css), one for one:
 * dark ink on warm cream by day, pale ink on soot-dark card stock by night.
 * What a person says is framed, what the machine wrote is filled, and the
 * one colour is a red pencil mark for what wants attention. Change a value
 * here and in index.css together.
 */
@Immutable
data class Paper(
    val ink: Color,
    val mid: Color,
    val soft: Color,
    val hair: Color,
    val paper: Color,
    val lift: Color,
    val said: Color,
    val block: Color,
    val hover: Color,
    val frame: Color,
    val edge: Color,
    val accent: Color,
    val dark: Boolean,
)

private val Day = Paper(
    ink = Color(0xFF2A2019),
    // Darkened with their hue held to read at 4.5:1 on paper, as index.css.
    mid = Color(0xFF554537),
    soft = Color(0xFF6E5D48),
    hair = Color(0xFFD2C2A8),
    paper = Color(0xFFE6D8C1),
    lift = Color(0xFFEDE2CF),
    said = Color(0xFFF3EEE6),
    block = Color(0xFFDACAB0),
    hover = Color(0xFFD0BFA3),
    frame = Color(0xFFBEA98A),
    edge = Color(0xFF3A2E25),
    accent = Color(0xFFA53E20),
    dark = false,
)

private val Night = Paper(
    ink = Color(0xFFEBE3D5),
    mid = Color(0xFFB0A390),
    soft = Color(0xFF8A7E70),
    hair = Color(0xFF38322B),
    paper = Color(0xFF171411),
    lift = Color(0xFF24201C),
    said = Color(0xFF1E1A17),
    block = Color(0xFF2E2924),
    hover = Color(0xFF3A332C),
    frame = Color(0xFF564C41),
    // index.css frames the person's words in --frame by night (dark:border-frame).
    edge = Color(0xFFA3968A),
    accent = Color(0xFFE08A62),
    dark = true,
)

val LocalPaper = staticCompositionLocalOf { Day }

/** The page's colours, as `Ink.paper` reads `var(--paper)`. */
val Ink: Paper
    @Composable get() = LocalPaper.current

/** The one green: a host that answers, a worker at work. --accent is for
 * what needs the person, and outranks it. */
val Active = Color(0xFF2FA76E)

/** The person's words' frame: --edge by day, --frame by night. */
val Paper.mine get() = if (dark) frame else edge

/** One face of Inter's variable font (res/font/inter.ttf), the web's
 * shipped sans, at a weight. */
private fun inter(weight: FontWeight) =
    Font(R.font.inter, weight, variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)))

/** Inter as the web ships it; the serif falls back as Chrome's does on
 * Android, to Noto Serif. */
object Faces {
    val sans = FontFamily(inter(FontWeight.Normal), inter(FontWeight.Medium), inter(FontWeight.SemiBold), inter(FontWeight.Bold))
    val serif = FontFamily.Serif
    val mono = FontFamily.Monospace
}

/** The web's type scale (index.css @theme), px there and sp here: every
 * size in the app is one of these. */
object TypeScale {
    const val caption = 12f
    const val label = 13f
    const val control = 14f
    const val body = 15f
    const val prose = 16f
    const val title = 20f
    const val heading = 28f
    const val display = 46f
}

/** Type in the page's sizes (px there, sp here). Body is 16 on 1.55. */
fun type(size: Float, line: Float = 1.55f, weight: FontWeight = FontWeight.Normal, family: FontFamily = Faces.sans, spacing: TextUnit = TextUnit.Unspecified) =
    TextStyle(fontSize = size.sp, lineHeight = (size * line).sp, fontWeight = weight, fontFamily = family, letterSpacing = spacing)

object Type {
    val body = type(TypeScale.prose)
    val small = type(TypeScale.control, 1.4f)
    val note = type(TypeScale.label, 1.4f)
    val label = type(TypeScale.caption, 1.3f)
    /** Section heads: caption semibold, wide-tracked capitals. */
    val head = type(TypeScale.caption, 1.3f, FontWeight.SemiBold, spacing = 0.05.em)
    val title = type(TypeScale.title, 1.25f, family = Faces.serif)
    val code = type(TypeScale.label, 1.5f, family = Faces.mono)
}

/** The web's radius scale (index.css @theme): every corner is one of these.
 * Cards and menus take xl, the composer and a person's message the pill,
 * the message's tail corner md. */
object Radii {
    val sm = 4.dp
    val md = 6.dp
    val lg = 8.dp
    val xl = 12.dp
    val xxl = 16.dp
    val pill = 24.dp
    val card = xl
    /** A card in a list of them: Work's and Pinned's (web's rounded-2xl). */
    val listed = xxl
    /** Half the bar's 52dp, so the composer's ends round as the bar's do. */
    val composer = 26.dp
    val bubble = pill
    val tail = md
}

private fun scheme(p: Paper) = (if (p.dark) darkColorScheme() else lightColorScheme()).copy(
    primary = p.ink,
    onPrimary = p.paper,
    secondary = p.mid,
    background = p.paper,
    onBackground = p.ink,
    surface = p.paper,
    onSurface = p.ink,
    surfaceVariant = p.lift,
    onSurfaceVariant = p.mid,
    surfaceContainerLowest = p.paper,
    surfaceContainerLow = p.lift,
    surfaceContainer = p.lift,
    surfaceContainerHigh = p.lift,
    surfaceContainerHighest = p.block,
    secondaryContainer = p.hover,
    onSecondaryContainer = p.ink,
    inverseSurface = p.ink,
    inverseOnSurface = p.paper,
    outline = p.frame,
    outlineVariant = p.hair,
    error = p.accent,
    scrim = Color.Black,
)

private val typography = Typography().let { t ->
    t.copy(
        bodyLarge = Type.body,
        bodyMedium = Type.small,
        bodySmall = Type.note,
        labelLarge = type(TypeScale.body, 1.3f),
        labelMedium = Type.note,
        labelSmall = Type.label,
        titleLarge = Type.title,
        titleMedium = type(TypeScale.prose, 1.3f, FontWeight.SemiBold),
        titleSmall = type(TypeScale.body, 1.35f, FontWeight.Medium),
        headlineSmall = type(TypeScale.prose, 1.3f, FontWeight.SemiBold),
    )
}

/** `theme` is system, light or dark. */
@Composable
fun LibriTheme(theme: String = "system", content: @Composable () -> Unit) {
    val dark = when (theme) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
    val paper = if (dark) Night else Day
    // The status and navigation bars' icons follow the picked theme.
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }
    }
    CompositionLocalProvider(LocalPaper provides paper) {
        MaterialTheme(colorScheme = scheme(paper), typography = typography, content = content)
    }
}

/** The talk's text at `px` of the page's pixels (16 the default), as the
 * web's pinch and Settings › Appearance size only the talk. */
@Composable
fun TalkSize(px: Int, content: @Composable () -> Unit) {
    val density = LocalDensity.current
    CompositionLocalProvider(LocalDensity provides Density(density.density, density.fontScale * px / 16f), content = content)
}
