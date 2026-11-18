package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.PassphraseRule
import com.nickalie.ngterm.android.ui.components.Banner
import com.nickalie.ngterm.android.ui.components.ConfirmDialog
import com.nickalie.ngterm.android.ui.components.KeyPlate
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.components.notAutofilled
import com.nickalie.ngterm.android.ui.onboarding.BusyButton
import com.nickalie.ngterm.android.ui.onboarding.Problem
import com.nickalie.ngterm.android.ui.onboarding.SecureWindow
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.ExportProtection

/** Where an export goes, as the screen's two actions name it. */
private enum class Target { File, Clipboard }

/**
 * Exporting the private key [fingerprint] names, after the device's owner confirmed it is them: what the exported
 * key is, the passphrase that encrypts it (typed twice, long enough for [rule], the exporter's own count of its
 * characters), or the explicit choice of none, which is confirmed again;
 * then Save to a file ([onSave]) or Copy to the clipboard ([onCopy]) with the protection chosen. [work] says how the
 * last one went; [onStart] hears each tap on either, so what an earlier one said goes before this one ends. The
 * passphrase lives only in this screen's memory, never in saved state; the window is secure while it is up.
 */
@Composable
fun ExportKeyScreen(
    fingerprint: String,
    work: ExportWork,
    rule: PassphraseRule,
    onSave: (ExportProtection) -> Unit,
    onCopy: (ExportProtection) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    draft: ExportDraft = ExportDraft(),
    onStart: () -> Unit = {},
) {
    SecureWindow()
    var passphrase by remember { mutableStateOf(draft.passphrase) }
    var again by remember { mutableStateOf(draft.again) }
    var unprotected by remember { mutableStateOf(draft.unprotected) }
    var checked by remember { mutableStateOf(false) }
    var asking by remember { mutableStateOf<Target?>(null) }
    val send = { target: Target, protection: ExportProtection ->
        if (target == Target.File) onSave(protection) else onCopy(protection)
    }
    val start = { target: Target ->
        onStart()
        when {
            unprotected -> asking = target
            rule.tooShort(passphrase) || passphrase != again -> checked = true
            else -> send(target, ExportProtection.Passphrase(passphrase))
        }
    }
    ScreenFrame(
        modifier = modifier,
        title = stringResource(R.string.export_title),
        exit = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back), onBack),
    ) {
        Banner(StatusTone.Attention, stringResource(R.string.export_warning))
        KeyPlate(
            text = fingerprint,
            label = stringResource(R.string.export_key_label),
            style = NgtermTheme.mono.small,
        )
        if (!unprotected) {
            Passphrases(
                passphrase = passphrase,
                again = again,
                rule = rule,
                checked = checked,
                enabled = !work.busy,
                onPassphrase = { passphrase = it },
                onAgain = { again = it },
            )
        }
        SwitchRow(
            label = stringResource(R.string.export_unprotected),
            note = stringResource(R.string.export_unprotected_note),
            checked = unprotected,
            onCheckedChange = { if (!work.busy) unprotected = it },
        )
        if (unprotected) Banner(StatusTone.Error, stringResource(R.string.export_unprotected_warning))
        ExportActions(work, onSave = { start(Target.File) }, onCopy = { start(Target.Clipboard) })
    }
    asking?.let { target ->
        ConfirmUnprotected(
            onConfirm = {
                asking = null
                send(target, ExportProtection.Unprotected)
            },
            onDismiss = { asking = null },
        )
    }
}

// the second question before a key leaves unencrypted; its button repeats the switch's words
@Composable
private fun ConfirmUnprotected(
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    ConfirmDialog(
        title = stringResource(R.string.export_unprotected_title),
        body = stringResource(R.string.export_unprotected_body),
        confirmLabel = stringResource(R.string.export_unprotected),
        onConfirm = onConfirm,
        onDismiss = onDismiss,
    )
}

// the passphrase and its repeat; once an export was tried ([checked]) what is wrong shows under the field it is about
@Composable
private fun Passphrases(
    passphrase: String,
    again: String,
    rule: PassphraseRule,
    checked: Boolean,
    enabled: Boolean,
    onPassphrase: (String) -> Unit,
    onAgain: (String) -> Unit,
) {
    var shown by remember { mutableStateOf(false) }
    val short = checked && rule.tooShort(passphrase)
    val differ = checked && !short && passphrase != again
    Column(verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm)) {
        SecretField(
            value = passphrase,
            onValueChange = onPassphrase,
            label = stringResource(R.string.export_passphrase_label),
            note =
                pluralStringResource(
                    if (short) R.plurals.export_passphrase_short else R.plurals.export_passphrase_note,
                    rule.minimum,
                    rule.minimum,
                ),
            isError = short,
            shown = shown,
            enabled = enabled,
            imeAction = ImeAction.Next,
            onShow = { shown = it },
        )
        SecretField(
            value = again,
            onValueChange = onAgain,
            label = stringResource(R.string.export_confirm_label),
            note = stringResource(R.string.export_passphrase_mismatch).takeIf { differ },
            isError = differ,
            shown = shown,
            enabled = enabled,
            imeAction = ImeAction.Done,
        )
    }
}

@Composable
private fun SecretField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    note: String?,
    isError: Boolean,
    shown: Boolean,
    enabled: Boolean,
    imeAction: ImeAction,
    modifier: Modifier = Modifier,
    onShow: ((Boolean) -> Unit)? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label) },
        singleLine = true,
        isError = isError,
        supportingText = note?.let { { Text(it) } },
        visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions =
            KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Password, imeAction = imeAction),
        trailingIcon =
            onShow?.let {
                {
                    TextButton(onClick = { it(!shown) }) {
                        Text(
                            stringResource(
                                if (shown) R.string.import_hide_passphrase else R.string.import_show_passphrase,
                            ),
                        )
                    }
                }
            },
        enabled = enabled,
        modifier = modifier.notAutofilled().fillMaxWidth(),
    )
}

// the two ways out, the safer one filled; then how the last export went
@Composable
private fun ExportActions(
    work: ExportWork,
    onSave: () -> Unit,
    onCopy: () -> Unit,
) {
    val spacing = NgtermTheme.spacing
    Column(Modifier.fillMaxWidth().padding(top = spacing.sm), verticalArrangement = Arrangement.spacedBy(spacing.xs)) {
        BusyButton(
            label = stringResource(if (work.busy) R.string.export_working else R.string.export_save),
            busy = work.busy,
            onClick = onSave,
            modifier = Modifier.fillMaxWidth(),
        )
        TextButton(onClick = onCopy, enabled = !work.busy) { Text(stringResource(R.string.export_copy)) }
        Text(
            stringResource(R.string.export_copy_note),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        work.done?.let {
            Banner(
                StatusTone.Online,
                stringResource(if (it == ExportDone.Saved) R.string.export_saved else R.string.export_copied),
                Modifier.padding(top = spacing.sm),
            )
        }
        work.problem?.let { Problem(it.words(), Modifier.padding(top = spacing.sm)) }
    }
}

@Composable
private fun ExportProblem.words(): String =
    when (this) {
        ExportProblem.Expired -> stringResource(R.string.export_expired)
        ExportProblem.Interrupted -> stringResource(R.string.export_interrupted)
        is ExportProblem.ShortPassphrase -> pluralStringResource(R.plurals.export_passphrase_short, minimum, minimum)
        ExportProblem.Gone -> stringResource(R.string.export_gone)
        is ExportProblem.Failed -> stringResource(R.string.export_failed, reason)
    }
