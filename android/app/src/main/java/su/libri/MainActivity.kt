package su.libri

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver

class MainActivity : ComponentActivity() {
    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        model.shared(intent)
    }

    private val model: ChatModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // The page colour runs under the navigation buttons, as on the web
        // app, rather than a white scrim.
        window.isNavigationBarContrastEnforced = false
        // Follow the host only while the app is in view; coming back reads
        // the conversation afresh, so nothing said meanwhile is missed.
        lifecycle.addObserver(
            LifecycleEventObserver { _, event ->
                when (event) {
                    Lifecycle.Event.ON_START -> { model.start(); Bell.shown = true }
                    Lifecycle.Event.ON_STOP -> { model.stop(); Bell.shown = false }
                    else -> {}
                }
            },
        )
        if (savedInstanceState == null) model.shared(intent)
        setContent { LibriTheme(model.theme) { LibriApp(model) } }
    }
}
