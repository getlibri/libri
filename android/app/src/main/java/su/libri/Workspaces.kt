package su.libri

import su.libri.core.PhonePane
import su.libri.core.PhoneSpace

// A phone's workspaces (phone.tsx FIRST, unnamed, fresh): what a first run
// opens on, new ones' names and ids, the same as the web's on a phone.

private var made = 0

/** An id no other workspace has. */
fun freshSpaceId(): String = "p" + System.currentTimeMillis().toString(36) + (made++).toString(36)

/** What a first run opens on: the bar as it stood before workspaces, Home
 * gone. And what closing the last one leaves. */
fun firstSpaces(): Pair<List<PhoneSpace>, String> {
    val w = PhoneSpace(freshSpaceId(), "Main", listOf(PhonePane("chat", null), PhonePane("library", null)))
    return listOf(w) to w.id
}

/** A bar without the Workers item, gone since the composer's agents took its
 * work (Agents.kt); a bar left with nothing holds the chat. */
fun withoutWorkers(spaces: List<PhoneSpace>): List<PhoneSpace> = spaces.map { w ->
    val items = w.items.filter { it.kind != "workers" }
    if (items.size == w.items.size) w else w.copy(items = items.ifEmpty { listOf(PhonePane("chat", null)) })
}

/** The first free name of the form `Workspace N`. */
fun unnamedSpace(names: List<String>): String =
    generateSequence(names.size + 1) { it + 1 }.map { "Workspace $it" }.first { it !in names }
