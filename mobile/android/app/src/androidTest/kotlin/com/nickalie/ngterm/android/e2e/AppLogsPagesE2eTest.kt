package com.nickalie.ngterm.android.e2e

import android.content.ClipboardManager
import android.content.Intent
import android.os.ParcelFileDescriptor
import android.util.Base64
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.logs.LogsViewModel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith
import java.io.File

/**
 * Settings > Logs and its own line, end to end: each time the page opens (from Settings, again after Back, and after
 * a rotation each way) its newest line is "settings page=logs", with no need to leave the app and resume it. Copy takes
 * the lines shown, with that line the newest. When the app comes back from the background, the page reads the log
 * again and writes no line of its own. The log that Share writes has one "settings page=" line for each page that
 * showed.
 *
 * The stack is [SoftKeyboardSwitchE2eTest]'s: `scripts/test-android-e2e.sh` starts the machine and hands it over as
 * instrumentation arguments; without them the test is skipped.
 */
@RunWith(AndroidJUnit4::class)
class AppLogsPagesE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()
    private val packageName = instrumentation.targetContext.packageName

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin, no exported log), cleared before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
                exportDir().deleteRecursively()
            }
        }

    /** The display turns back to its natural orientation and rotates freely again, whatever the test left. */
    private val orientation =
        object : ExternalResource() {
            override fun after() {
                device.setOrientationNatural()
                device.unfreezeRotation()
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) = screenshot("failed")
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstall)
            .around(orientation)
            .around(compose)
            .around(evidence)

    @Test
    fun theLogsPageShowsItsOwnLineAsTheNewestEachTimeItOpens() {
        signInAndListSessions(String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT)))

        compose.onAllNodesWithText(text(R.string.nav_settings))[0].performClick()
        openLogs()
        awaitNewestIsTheLogsPage("first")

        // Back to Settings, then Logs again
        compose.onAllNodesWithContentDescription(text(R.string.action_back))[0].performClick()
        openLogs()
        awaitNewestIsTheLogsPage("again")

        // a rotation each way makes the activity again, and the page comes back
        device.setOrientationLeft()
        awaitNewestIsTheLogsPage("landscape")
        device.setOrientationNatural()
        awaitNewestIsTheLogsPage("portrait")

        // Copy takes the lines shown, the page's own line the newest of them
        compose.onNodeWithText(text(R.string.logs_copy)).performClick()
        awaitText(text(R.string.action_copied))
        val copied = copiedText().lines().filter { it.isNotBlank() }
        assertTrue("the newest copied line is the page's own:\n${copied.last()}", copied.last().endsWith(LOGS_LINE))

        // the app goes to the background and comes back: the page reads the log again, which now ends with the app's
        // own lines of that, and writes no page line of its own
        val returns = foregroundLinesShown()
        device.pressHome()
        assertTrue("the app left the screen", device.wait(Until.gone(By.pkg(packageName).depth(0)), WAIT_MS))
        val launch = requireNotNull(instrumentation.targetContext.packageManager.getLaunchIntentForPackage(packageName))
        instrumentation.targetContext.startActivity(launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        assertTrue("the app came back", device.wait(Until.hasObject(By.pkg(packageName).depth(0)), WAIT_MS))
        compose.waitUntil(WAIT_MS) { foregroundLinesShown() > returns }
        screenshot("resumed")

        // one line for each page that showed: Settings, Logs, Settings, Logs, then Logs after each rotation, and none
        // after the return from the background
        compose.onNodeWithText(text(R.string.logs_share)).performClick()
        compose.waitUntil(WAIT_MS) { exported() != null }
        compose.waitForIdle()
        val pages =
            requireNotNull(exported())
                .readLines()
                .filter { PAGE in it }
                .map { it.substringAfter(PAGE) }
        assertEquals(listOf("home", "logs", "home", "logs", "logs", "logs"), pages)
        device.wait(Until.gone(By.pkg(packageName).depth(0)), WAIT_MS)
        device.pressBack()
    }

    private fun openLogs() {
        awaitText(text(R.string.settings_logs))
        compose.onNodeWithText(text(R.string.settings_logs)).performScrollTo().performClick()
        awaitText(text(R.string.logs_never))
    }

    // the newest line is the first one on the plate, under the day's date
    private fun awaitNewestIsTheLogsPage(shot: String) {
        var newest: String? = null
        try {
            compose.waitUntil(NEWEST_WAIT_MS) {
                newest = newestShownLine()
                newest?.endsWith(LOGS_LINE) == true
            }
        } catch (e: ComposeTimeoutException) {
            throw AssertionError("the newest line shown ($shot) is \"$newest\", not \"$LOGS_LINE\"", e)
        }
        screenshot(shot)
    }

    private fun newestShownLine(): String? = shownLog()?.lines()?.firstOrNull { it.isNotBlank() && !DAY.matches(it) }

    private fun foregroundLinesShown(): Int = shownLog()?.lines()?.count { FOREGROUND_LINE in it } ?: 0

    // the plate of log lines, newest first
    private fun shownLog(): String? =
        compose
            .onAllNodes(hasText(PAGE, substring = true))
            .fetchSemanticsNodes()
            .firstNotNullOfOrNull { node -> node.config.getOrNull(SemanticsProperties.Text)?.joinToString("") }

    private fun signInAndListSessions(key: String) {
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(MACHINE)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
    }

    private fun copiedText(): String {
        val clipboard = instrumentation.targetContext.getSystemService(ClipboardManager::class.java)
        var text = ""
        instrumentation.runOnMainSync {
            text =
                clipboard.primaryClip
                    ?.getItemAt(0)
                    ?.text
                    ?.toString()
                    .orEmpty()
        }
        return text
    }

    private fun exportDir() = File(instrumentation.targetContext.cacheDir, LogsViewModel.EXPORT_DIR)

    private fun exported() = exportDir().walk().firstOrNull { it.name == LogsViewModel.EXPORT_NAME }

    private fun awaitText(text: String) =
        compose.waitUntil(WAIT_MS) { compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }

    private fun text(id: Int): String = instrumentation.targetContext.getString(id)

    private fun screenshot(name: String) {
        ParcelFileDescriptor
            .AutoCloseInputStream(
                instrumentation.uiAutomation.executeShellCommand("screencap -p $EVIDENCE_DIR/app-logs-pages-$name.png"),
            ).bufferedReader()
            .use { it.readText() }
    }

    private companion object {
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"
        const val MACHINE = "qa-pages"
        const val PAGE = "settings page="
        const val LOGS_LINE = "settings page=logs"
        const val FOREGROUND_LINE = "app in the foreground"
        val DAY = Regex("""\d{4}-\d{2}-\d{2}""")

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val WAIT_MS = 60_000L

        // the page reads the log once as it opens: its own line is there at once or never
        const val NEWEST_WAIT_MS = 10_000L
    }
}
