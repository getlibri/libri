package su.libri

import java.text.DateFormat
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

/** One nap the menu offers: its words and when it wakes, in seconds. */
data class NapPreset(val label: String, val until: Long)

private fun clock(c: Calendar): String = DateFormat.getTimeInstance(DateFormat.SHORT).format(c.time)

private fun at(day: Calendar, hour: Int) = (day.clone() as Calendar).apply {
    set(Calendar.HOUR_OF_DAY, hour); set(Calendar.MINUTE, 0); set(Calendar.SECOND, 0); set(Calendar.MILLISECOND, 0)
}

/** The naps to offer at `now`, soonest first: T3 Code's five snooze presets,
 * those already past left out, two in the same minute shown once (nap.ts). */
fun napPresets(now: Calendar = Calendar.getInstance()): List<NapPreset> {
    val tomorrow = (now.clone() as Calendar).apply { add(Calendar.DATE, 1) }
    // Next Monday, a week on when today is one.
    val ahead = ((Calendar.MONDAY - now.get(Calendar.DAY_OF_WEEK) + 7) % 7).let { if (it == 0) 7 else it }
    val monday = (now.clone() as Calendar).apply { add(Calendar.DATE, ahead) }
    val inHours = { h: Int -> (now.clone() as Calendar).apply { add(Calendar.HOUR_OF_DAY, h) } }
    val all = listOf(
        "1 hour" to inHours(1),
        "3 hours" to inHours(3),
        "This evening, ${clock(at(now, 18))}" to at(now, 18),
        "Tomorrow, ${clock(at(tomorrow, 9))}" to at(tomorrow, 9),
        "Monday, ${clock(at(monday, 9))}" to at(monday, 9),
    )
    val seen = mutableSetOf<Long>()
    return all.filter { it.second.timeInMillis > now.timeInMillis }
        .sortedBy { it.second.timeInMillis }
        .filter { seen.add(it.second.timeInMillis / 60_000) }
        .map { NapPreset(it.first, it.second.timeInMillis / 1000) }
}

/** When a nap wakes, as a card's few words: the hour today, else the day (nap.ts wakeWords). */
fun wakeWords(until: Long, now: Calendar = Calendar.getInstance()): String {
    val wake = Calendar.getInstance().apply { timeInMillis = until * 1000 }
    val days = ((at(wake, 0).timeInMillis - at(now, 0).timeInMillis + 43_200_000) / 86_400_000).toInt()
    return when {
        days <= 0 -> clock(wake)
        days == 1 -> "tomorrow ${clock(wake)}"
        days < 7 -> "${SimpleDateFormat("EEE", Locale.getDefault()).format(wake.time)} ${clock(wake)}"
        else -> "${SimpleDateFormat("MMM d", Locale.getDefault()).format(Date(until * 1000))} ${clock(wake)}"
    }
}
