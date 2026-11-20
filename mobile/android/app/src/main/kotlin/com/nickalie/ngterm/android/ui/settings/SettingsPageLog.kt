package com.nickalie.ngterm.android.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import com.nickalie.ngterm.android.data.Diagnostics
import com.nickalie.ngterm.android.data.LogTag

/**
 * Tells the log which page Settings shows, by its kind alone, one time for each change of the page. Settings writes it
 * in the composition, before the page composes: a page that reads the log (Logs) finds its own line there.
 */
internal class SettingsPageLog {
    // the page the log was last told
    private var noted: String? = null

    /** Tells the log that Settings shows the page [shown], if it did not show it before. */
    fun note(shown: String) {
        if (shown == noted) return
        noted = shown
        Diagnostics.info(LogTag.NAV, "settings page=$shown")
    }
}

/**
 * The log of Settings' pages. It starts again after a rotation or a restore, so the page that shows then is logged
 * again.
 */
@Composable
internal fun rememberSettingsPageLog(): SettingsPageLog = remember { SettingsPageLog() }
