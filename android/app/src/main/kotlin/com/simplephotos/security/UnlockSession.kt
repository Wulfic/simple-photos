/**
 * Process-scoped app-lock state (#21 / #17).
 *
 * The biometric gate used to live entirely in a `remember` inside MainActivity,
 * i.e. it was per-activity-instance. Split-screen opens a SECOND MainActivity
 * (see ui/navigation/NewWindow.kt), which made the gate demand a fingerprint
 * again seconds after the user unlocked the first window — same process, same
 * session, two prompts.
 *
 * The unlock is a property of the *process*, which is the actual trust
 * boundary: both windows already share one Room DB, one auth token and one
 * decrypted cache, so a second prompt guards nothing. Hoisting it here means
 * the second window inherits the unlock, while a true process death still
 * cold-starts locked — preserving the contract documented for #17.
 */
package com.simplephotos.security

import javax.inject.Inject
import javax.inject.Singleton

/**
 * Whether the app-lock has been satisfied in this process.
 *
 * Deliberately holds no reset/lock method: nothing re-locks a live process
 * today (backgrounding doesn't, and rotation must not — see #17), and the
 * singleton dies with the process, which is exactly the intended lifetime.
 */
@Singleton
class UnlockSession @Inject constructor() {

    @Volatile
    var isUnlocked: Boolean = false
        private set

    /** Record that the user satisfied the app-lock (biometric or device PIN). */
    fun markUnlocked() {
        isUnlocked = true
    }
}
