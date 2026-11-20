package com.nickalie.ngterm.android.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.Stable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import com.nickalie.ngterm.android.data.Diagnostics
import com.nickalie.ngterm.android.data.LogTag
import kotlin.reflect.KProperty

/**
 * The page Settings shows, kept in [saved], which tells the log each page by its kind alone as it is set, before the
 * page composes: a page that reads the log (Logs) finds its own line there.
 */
@Stable
internal class SettingsPage(
    private val saved: MutableState<String>,
) {
    // the page the log was last told
    private var noted: String? = null

    operator fun getValue(
        owner: Any?,
        property: KProperty<*>,
    ): String = saved.value

    operator fun setValue(
        owner: Any?,
        property: KProperty<*>,
        value: String,
    ) {
        saved.value = value
        note(value.substringBefore(':'))
    }

    /** Tells the log that Settings shows the page [shown], once. */
    fun note(shown: String) {
        if (shown == noted) return
        noted = shown
        Diagnostics.info(LogTag.NAV, "settings page=$shown")
    }
}

/** Settings' page, [first] at first, kept over a rotation and a process's restart. */
@Composable
internal fun rememberSettingsPage(first: String): SettingsPage {
    val saved = rememberSaveable { mutableStateOf(first) }
    return remember(saved) { SettingsPage(saved) }
}
