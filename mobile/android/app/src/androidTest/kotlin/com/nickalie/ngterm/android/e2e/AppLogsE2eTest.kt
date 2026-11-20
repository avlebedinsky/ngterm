package com.nickalie.ngterm.android.e2e

import android.content.ClipboardManager
import android.os.ParcelFileDescriptor
import android.util.Base64
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
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
 * The diagnostic log, end to end: after signing in to a real machine and reading its sessions, Settings > Logs shows
 * the connection's lines (the app's and the core's, written through the real native log) and the page's own line,
 * written as it opens; Copy puts them on the clipboard, and Share writes the whole log to a file and opens the
 * system's share sheet. Neither the imported private key nor the host key's fingerprint is in what was copied or
 * exported.
 *
 * The stack is [SoftKeyboardSwitchE2eTest]'s: `scripts/test-android-e2e.sh` starts the machine and hands it over as
 * instrumentation arguments; without them the test is skipped.
 */
@RunWith(AndroidJUnit4::class)
class AppLogsE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()

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
            .around(compose)
            .around(evidence)

    @Test
    fun theLogShowsAConnectAndIsCopiedAndShared() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        signInAndListSessions(key)

        compose.onAllNodesWithText(text(R.string.nav_settings))[0].performClick()
        awaitText(text(R.string.settings_logs))
        compose.onNodeWithText(text(R.string.settings_logs)).performScrollTo().performClick()
        awaitText(text(R.string.logs_never))
        // the page's own line, written as it opens, shows without leaving the app and coming back
        awaitText("settings page=logs", substring = true)
        for (line in CONNECT_LINES) awaitText(line, substring = true)
        screenshot("shown")

        compose.onNodeWithText(text(R.string.logs_copy)).performClick()
        awaitText(text(R.string.action_copied))
        val copied = copiedText()
        for (line in CONNECT_LINES) assertTrue("the copied lines hold \"$line\":\n$copied", line in copied)
        assertClean(copied, key)

        compose.onNodeWithText(text(R.string.logs_share)).performClick()
        // the test's clock draws the frames that hand the file over, so they are waited for through it
        compose.waitUntil(SHEET_WAIT_MS) { exported() != null }
        compose.waitForIdle()
        // the share sheet is the system's: the app is no longer the window on top
        val left = device.wait(Until.gone(By.pkg(instrumentation.targetContext.packageName).depth(0)), SHEET_WAIT_MS)
        screenshot("share-sheet")
        assertNotNull("the share sheet opened", left)
        assertTrue("the share sheet opened", left)
        val whole = requireNotNull(exported()).readText()
        for (line in CONNECT_LINES) assertTrue("the exported log holds \"$line\"", line in whole)
        assertTrue("the exported log holds the copy too", "logs: copied the shown lines" in whole)
        assertClean(whole, key)
        device.pressBack()
    }

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

    // neither the private key's body nor a host key fingerprint
    private fun assertClean(
        log: String,
        key: String,
    ) {
        for (line in key.lines().filter { it.length > KEY_LINE }) {
            assertFalse("the log holds key material", line in log)
        }
        assertFalse("the log holds a fingerprint", "SHA256:" in log)
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

    private fun awaitText(
        text: String,
        substring: Boolean = false,
    ) = compose.waitUntil(CONNECT_WAIT_MS) {
        compose.onAllNodes(hasText(text, substring = substring)).fetchSemanticsNodes().isNotEmpty()
    }

    private fun text(id: Int): String = instrumentation.targetContext.getString(id)

    private fun screenshot(name: String) {
        ParcelFileDescriptor
            .AutoCloseInputStream(
                instrumentation.uiAutomation.executeShellCommand("screencap -p $EVIDENCE_DIR/app-logs-$name.png"),
            ).bufferedReader()
            .use { it.readText() }
    }

    private companion object {
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"
        const val MACHINE = "qa-machine"

        /** What a connect and a listing leave in the log: the core's lines and the app's. */
        val CONNECT_LINES =
            listOf(
                "connect asked",
                "label=$MACHINE",
                "host key trusted and pinned",
                "authenticating",
                "method=publickey",
                "authenticated",
                "machine connected",
                "listing asked",
                "listing answered",
                "peer=ngterm",
            )

        const val KEY_LINE = 16
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val SHEET_WAIT_MS = 15_000L
    }
}
