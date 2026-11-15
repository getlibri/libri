package su.libri

// A worker's model by the name the model picker shows ("Opus 5.5"), whatever
// form its pick was stored in (modelNames.ts): the full id, the id without
// its maker's prefix or date, or a harness's alias ("opus", which Claude Code
// takes for its newest Opus). Until the offers arrive, or for an alias
// nothing on offer matches, the model shows as stored.

import su.libri.core.Offer

/** An id as harnesses vary it: no provider path, maker prefix or date. */
private fun bare(id: String) = id.substringAfterLast('/').lowercase().removePrefix("claude-").replace(Regex("[-@]\\d{8}$"), "")

private fun version(id: String) = Regex("\\d+").findAll(bare(id)).map { it.value.toInt() }.toList()

private val newestFirst = Comparator<Offer> { a, b ->
    val x = version(a.model)
    val y = version(b.model)
    (0 until maxOf(x.size, y.size)).map { (y.getOrElse(it) { 0 }) - (x.getOrElse(it) { 0 }) }.firstOrNull { it != 0 } ?: 0
}

/** The offer a stored model stands for: an exact one, then one equal once
 * both are bare, then for an alias (a word with no version) the newest of
 * that family on the harness. None for an empty model. */
fun offerOf(harness: String, model: String, offers: List<Offer>): Offer? {
    if (model.isEmpty()) return null
    val mine = offers.filter { it.harness == harness }
    mine.firstOrNull { it.model == model }?.let { return it }
    mine.firstOrNull { bare(it.model) == bare(model) }?.let { return it }
    val alias = bare(model)
    if (!Regex("^[a-z]+$").matches(alias)) return null
    return mine.filter { bare(it.model).startsWith("$alias-") }.sortedWith(newestFirst).firstOrNull()
}

/** The display name of `model` on `harness`; an empty model is the harness's
 * default; else the model as stored, bare. */
fun modelName(harness: String, model: String, offers: List<Offer>): String =
    if (model.isEmpty()) offers.firstOrNull { it.harness == harness && it.preferred }?.name.orEmpty()
    else offerOf(harness, model, offers)?.name ?: bare(model)
