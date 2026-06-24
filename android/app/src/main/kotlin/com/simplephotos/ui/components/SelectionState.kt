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

    /** Clear the selection and exit selection mode. */
    fun clear() {
        selectedIds = emptySet()
        isSelectionMode = false
    }
}
