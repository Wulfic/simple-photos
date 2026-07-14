/**
 * Reusable multi-select state machine for grid screens.
 *
 * Gallery, AlbumDetail, and Trash ViewModels each hand-rolled the identical
 * `selectedIds` + `isSelectionMode` pair and the enter/toggle/clear transitions.
 * This is the single source of truth — the Android counterpart of the web
 * `usePhotoSelection` hook.
 *
 * Each ViewModel keeps a private instance and re-exposes `selectedIds` /
 * `isSelectionMode` + its selection methods by delegating here, so the screens
 * that read `viewModel.selectedIds` etc. are unaffected.
 */
package com.simplephotos.ui.components

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * Pure set-math for toggling a whole group of ids (e.g. every photo in a day).
 * Extracted from [SelectionState] so it can be unit-tested without the Compose
 * runtime. If every id in [group] is already present in [current], the group is
 * removed (deselect the whole group); otherwise it is added. An empty [group]
 * is a no-op. Mirrors the web `toggleSelectGroup` (#24).
 */
fun toggleGroupSelection(current: Set<String>, group: Set<String>): Set<String> {
    if (group.isEmpty()) return current
    val allSelected = group.all { it in current }
    return if (allSelected) current - group else current + group
}

class SelectionState {
    var selectedIds by mutableStateOf(emptySet<String>())
        private set
    var isSelectionMode by mutableStateOf(false)
        private set

    /** Enter selection mode with [id] as the only selected item. */
    fun enter(id: String) {
        isSelectionMode = true
        selectedIds = setOf(id)
    }

    /**
     * Toggle [id] in/out of the selection. Exits selection mode when the set
     * becomes empty. No-op when not already in selection mode (preserves the
     * original per-screen guard).
     */
    fun toggle(id: String) {
        if (!isSelectionMode) return
        selectedIds = if (id in selectedIds) selectedIds - id else selectedIds + id
        if (selectedIds.isEmpty()) isSelectionMode = false
    }

    /** Enter selection mode and replace the selection with [ids]. */
    fun setSelection(ids: Set<String>) {
        isSelectionMode = true
        selectedIds = ids
    }

    /**
     * Toggle a whole group of ids at once (e.g. every photo in a day). If the
     * group is already fully selected it is removed, otherwise it is added.
     * Exits selection mode when the result is empty (parity with [toggle]) so a
     * fully-deselected day doesn't strand an empty selection bar (#24).
     */
    fun toggleGroup(ids: Set<String>) {
        val next = toggleGroupSelection(selectedIds, ids)
        if (next.isEmpty()) {
            selectedIds = emptySet()
            isSelectionMode = false
        } else {
            selectedIds = next
            isSelectionMode = true
        }
    }

    /** Clear the selection and exit selection mode. */
    fun clear() {
        selectedIds = emptySet()
        isSelectionMode = false
    }
}
