package com.simplephotos.ui.screens.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import com.simplephotos.ui.components.SpButton
import com.simplephotos.ui.components.SpButtonVariant
import com.simplephotos.ui.screens.settings.FixDatesViewModel.UiState
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

private val DAY = DateTimeFormatter.ofPattern("d MMM yyyy")

private fun day(ms: Long?): String = when {
    ms == null -> "unknown"
    ms <= 0L -> "1970 (no date)"
    else -> DAY.format(Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault()))
}

/** Settings → Photo dates (TODO §4b P1c). */
@Composable
internal fun FixDatesCard(viewModel: FixDatesViewModel = hiltViewModel()) {
    SettingsCard(title = "Photo dates", icon = Icons.Default.DateRange) {
        Text(
            "Downloaded and saved photos backed up before this update may sort at 1970. " +
                "Check shows how many would move to the day they landed on this phone — " +
                "nothing changes until you tap Apply.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        SpButton(
            "Check photo dates",
            onClick = viewModel::check,
            variant = SpButtonVariant.Secondary,
            fontSize = 14,
            modifier = Modifier.fillMaxWidth(),
        )
        if (viewModel.hasUndo) {
            Spacer(Modifier.height(8.dp))
            SpButton(
                "Undo last fix",
                onClick = viewModel::checkUndo,
                variant = SpButtonVariant.Secondary,
                fontSize = 14,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }

    when (val s = viewModel.state) {
        UiState.Idle -> Unit
        is UiState.Working -> AlertDialog(
            onDismissRequest = {},
            title = { Text("Photo dates") },
            text = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(12.dp))
                    Text(s.message)
                }
            },
            confirmButton = {},
        )
        is UiState.Preview -> PreviewDialog(s.preview, onApply = viewModel::apply, onCancel = viewModel::dismiss)
        is UiState.UndoConfirm -> AlertDialog(
            onDismissRequest = viewModel::dismiss,
            title = { Text("Undo last fix?") },
            text = {
                Text(
                    if (s.preview.counts.change > 0)
                        "${s.preview.counts.change} photos will go back to their previous dates." +
                            if (s.preview.counts.stale > 0) " ${s.preview.counts.stale} changed since and will be left as they are." else ""
                    else "Nothing left to undo — those photos were changed since."
                )
            },
            confirmButton = {
                if (s.preview.counts.change > 0) Button(onClick = viewModel::undo) { Text("Undo ${s.preview.counts.change}") }
            },
            dismissButton = { TextButton(onClick = viewModel::dismiss) { Text("Cancel") } },
        )
        is UiState.Done -> AlertDialog(
            onDismissRequest = viewModel::dismiss,
            title = { Text(s.title) },
            text = { Text(s.message) },
            confirmButton = { Button(onClick = viewModel::dismiss) { Text("OK") } },
        )
        is UiState.Failed -> AlertDialog(
            onDismissRequest = viewModel::dismiss,
            title = { Text("Couldn't check photo dates") },
            text = { Text("Nothing was changed by the step that failed. ${s.message}") },
            confirmButton = { Button(onClick = viewModel::dismiss) { Text("OK") } },
        )
    }
}

@Composable
private fun PreviewDialog(
    p: com.simplephotos.data.repository.DateRepairRepository.Preview,
    onApply: () -> Unit,
    onCancel: () -> Unit,
) {
    val n = p.toApply.size
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(if (n > 0) "$n photos will move" else "Nothing to fix") },
        text = {
            Column(
                Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                if (n > 0) {
                    Text("${p.plan.atEpoch} of them are at 1970 today.", style = MaterialTheme.typography.bodyMedium)
                    Spacer(Modifier.height(8.dp))
                    Text("Examples", fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleSmall)
                    for (smp in p.samples) {
                        Text(
                            "${smp.filename}\n  ${day(smp.fromMs)} → ${day(smp.toMs)}",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    if (n > p.samples.size) Text("…and ${n - p.samples.size} more", style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(8.dp))
                }
                Text("Left alone", fontWeight = FontWeight.SemiBold, style = MaterialTheme.typography.titleSmall)
                val skipped = listOf(
                    p.plan.alreadyRight to "already have the right date",
                    p.plan.notOnPhone to "not from this phone",
                    p.plan.fileGone to "no longer on this phone",
                    p.plan.noUsableDate to "have no usable date on this phone",
                    p.plan.serverDateUnreadable to "have an unreadable date on the server",
                    p.server.protected to "keep the date from their original file",
                    p.server.stale to "changed on the server since",
                ).filter { it.first > 0 }
                for ((count, why) in skipped) {
                    Text("$count $why", style = MaterialTheme.typography.bodySmall)
                }
                if (n > 0) {
                    Spacer(Modifier.height(8.dp))
                    Text(
                        "You can undo this from Settings afterwards.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        confirmButton = {
            Button(onClick = if (n > 0) onApply else onCancel) { Text(if (n > 0) "Apply to $n" else "OK") }
        },
        dismissButton = { if (n > 0) TextButton(onClick = onCancel) { Text("Cancel") } },
    )
}
