package com.simplephotos.ui.screens.settings

import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.simplephotos.data.remote.dto.DateRepairCounts
import com.simplephotos.data.repository.DateRepairRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.launch
import javax.inject.Inject

/**
 * Settings → Photo dates (TODO §4b P1c). Check is always a dry run; nothing is
 * written until the owner taps Apply on the preview that names the count.
 */
@HiltViewModel
class FixDatesViewModel @Inject constructor(
    private val repo: DateRepairRepository,
) : ViewModel() {

    sealed interface UiState {
        data object Idle : UiState
        data class Working(val message: String) : UiState
        data class Preview(val preview: DateRepairRepository.Preview) : UiState
        data class UndoConfirm(val preview: DateRepairRepository.UndoPreview) : UiState
        data class Done(val title: String, val message: String) : UiState
        data class Failed(val message: String) : UiState
    }

    var state by mutableStateOf<UiState>(UiState.Idle)
        private set
    var hasUndo by mutableStateOf(false)
        private set

    init { refreshUndo() }

    private fun refreshUndo() {
        viewModelScope.launch {
            hasUndo = runCatching { repo.hasUndo() }.getOrDefault(false)
        }
    }

    fun check() = run("check") {
        state = UiState.Preview(repo.preview { state = UiState.Working(it) })
    }

    fun apply() {
        val p = (state as? UiState.Preview)?.preview ?: return
        if (p.toApply.isEmpty()) { dismiss(); return }
        run("apply") {
            val r = repo.apply(p) { state = UiState.Working(it) }
            state = UiState.Done("Photo dates fixed", summary(r.counts, applied = true))
        }
    }

    fun checkUndo() = run("undo-check") {
        state = UiState.Working("Checking the last fix…")
        state = UiState.UndoConfirm(repo.previewUndo())
    }

    fun undo() = run("undo") {
        val c = repo.undoLast { state = UiState.Working(it) }
        state = UiState.Done("Last fix undone", "${c.change} photos went back to their old dates." +
            if (c.stale > 0) " ${c.stale} were changed since and were left as they are." else "")
    }

    fun dismiss() {
        if (state is UiState.Working) return
        state = UiState.Idle
        refreshUndo()
    }

    private fun run(what: String, block: suspend () -> Unit) {
        if (state is UiState.Working) return
        state = UiState.Working("Starting…")
        viewModelScope.launch {
            try {
                block()
            } catch (e: Exception) {
                Log.e("FixDatesViewModel", "$what failed", e)
                state = UiState.Failed(e.message ?: e.javaClass.simpleName)
            } finally {
                refreshUndo()
            }
        }
    }

    companion object {
        fun summary(c: DateRepairCounts, applied: Boolean): String = buildString {
            append(if (applied) "${c.change} photos moved to the right date." else "${c.change} photos will move.")
            if (c.stale > 0) append(" ${c.stale} changed since the check and were skipped.")
            if (c.protected > 0) append(" ${c.protected} keep the date from their original file.")
            if (c.notFound > 0) append(" ${c.notFound} are no longer on the server.")
        }
    }
}
