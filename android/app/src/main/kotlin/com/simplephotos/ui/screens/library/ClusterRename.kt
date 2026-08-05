/**
 * Renaming a person / pet cluster (#39), as a pure function.
 *
 * `PetDetailViewModel` and `PersonDetailViewModel` cannot be unit-tested
 * directly — both take `PhotoRepository`, a concrete class with its own
 * dependency graph, so constructing one in a JVM test means standing up half
 * the app. The decision that actually matters (blank input, optimistic label,
 * do NOT keep the new label when the request failed) is therefore lifted out
 * here, where it takes a suspend lambda instead of a repository and a test can
 * drive every branch. Same move `RenditionChoice.kt` made for #49.
 *
 * Deliberately Android-free: no `android.util.Log`, no Compose. Logging lives
 * in the ViewModel adapter, which is trivially correct; this file stays a thing
 * a plain JVM test can exercise.
 */
package com.simplephotos.ui.screens.library

/** What a rename attempt did. Exhaustive on purpose — the caller must decide
 *  what happens on failure rather than defaulting to "assume it worked". */
sealed interface RenameOutcome {
    /** Input was blank; no request was made and nothing changed. */
    data object Skipped : RenameOutcome

    /** The server accepted [label]; the caller may show it. */
    data class Renamed(val label: String) : RenameOutcome

    /** The request threw. The caller must keep the OLD label. */
    data class Failed(val message: String) : RenameOutcome
}

/**
 * Trim [input], and unless it is blank hand it to [rename].
 *
 * The blank guard is the one that protects data: without it a user who clears
 * the field (or types only spaces) sends an empty name and wipes the cluster's
 * label. The dialog's Save button is also disabled while blank, but that is a
 * UI affordance and this is the rule.
 *
 * Note what is deliberately NOT here: a "renaming to the same string is a
 * no-op, skip the request" short-circuit. It looks like a free optimisation and
 * it is wrong for pets — `PetDetailViewModel.label` falls back to the species
 * when the cluster has no label of its own, so a pet displayed as "Dog" with a
 * NULL stored label would have a genuine rename to "Dog" silently dropped. The
 * displayed label is not the stored one, so it cannot be compared against.
 */
suspend fun performClusterRename(
    input: String,
    rename: suspend (String) -> Unit,
): RenameOutcome {
    val trimmed = input.trim()
    if (trimmed.isEmpty()) return RenameOutcome.Skipped
    return try {
        rename(trimmed)
        RenameOutcome.Renamed(trimmed)
    } catch (e: Exception) {
        // `e.message` alone is not enough: plenty of exceptions carry a null
        // message, and the existing person-rename path assigned it straight to
        // the error banner, so those failures surfaced as an empty error.
        RenameOutcome.Failed(e.message ?: e::class.java.simpleName)
    }
}
