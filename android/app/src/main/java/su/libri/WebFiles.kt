package su.libri

import android.app.DownloadManager
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Base64
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.widget.Toast
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import org.json.JSONObject

// Files in and out of the host's pages shown in a web view (HostWebView):
// what the page saves (an export, a picture) goes to Downloads, and a page's
// file input (an image for a whiteboard) opens the system's picker. A web
// view does neither by itself.

/** A page's `{"save": {name, type, data}}` message (site save.ts), saved to
 * Downloads; whether the message was one. */
fun savedFromPage(context: Context, message: String): Boolean {
    if (!message.startsWith("{")) return false
    val save = runCatching { JSONObject(message).optJSONObject("save") }.getOrNull() ?: return false
    val name = save.optString("name").ifBlank { "Libri file" }.replace(Regex("[/\\\\]"), "_")
    // The type its name says, so Downloads keeps the name as it is (it adds
    // an extension of its own for a type that names another, as
    // `.excalidraw.json`); one Android doesn't know is plain bytes.
    val type = android.webkit.MimeTypeMap.getSingleton()
        .getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
        ?: "application/octet-stream"
    val bytes = runCatching { Base64.decode(save.optString("data"), Base64.DEFAULT) }.getOrNull() ?: return true
    val saved = runCatching {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, type)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: error("no place in Downloads")
        resolver.openOutputStream(uri)?.use { it.write(bytes) } ?: error("Downloads can't be written")
        values.clear()
        values.put(MediaStore.Downloads.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
    }
    saved.exceptionOrNull()?.let { android.util.Log.w("libri", "saving $name: $it") }
    Toast.makeText(context, if (saved.isSuccess) "Saved $name to Downloads" else "Couldn't save $name", Toast.LENGTH_SHORT).show()
    return true
}

/** A download a page links to at an address (not a page's own `blob:`), as
 * a browser would fetch it, into Downloads. */
fun downloadsLinks(view: WebView) {
    view.setDownloadListener { url, userAgent, disposition, type, _ ->
        if (!url.startsWith("http")) return@setDownloadListener
        val name = URLUtil.guessFileName(url, disposition, type)
        val request = DownloadManager.Request(Uri.parse(url))
            .setMimeType(type)
            .addRequestHeader("User-Agent", userAgent)
            .setTitle(name)
            .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
            .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
        view.context.getSystemService(DownloadManager::class.java).enqueue(request)
        Toast.makeText(view.context, "Downloading $name", Toast.LENGTH_SHORT).show()
    }
}

/** The pending answer to a page's file input, kept across the picker. */
class FileAsk {
    var callback: ValueCallback<Array<Uri>>? = null
    lateinit var launcher: ManagedActivityResultLauncher<Array<String>, List<Uri>>
}

/** A file picker for the page's `<input type=file>`: the system's document
 * picker, which also offers the camera's recent photos and Drive. */
@Composable
fun rememberFileAsk(): FileAsk {
    val ask = remember { FileAsk() }
    ask.launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        ask.callback?.onReceiveValue(uris.toTypedArray())
        ask.callback = null
    }
    return ask
}

/** The web view's chrome, answering file inputs through `ask`. */
fun pickingChrome(ask: FileAsk) = object : WebChromeClient() {
    override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
        // One question at a time: an earlier one left open is answered empty.
        ask.callback?.onReceiveValue(null)
        ask.callback = callback
        val types = params.acceptTypes.filter { it.isNotBlank() }.ifEmpty { listOf("*/*") }
        runCatching { ask.launcher.launch(types.toTypedArray()) }.onFailure {
            ask.callback = null
            callback.onReceiveValue(null)
        }
        return true
    }
}
