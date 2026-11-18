package com.nickalie.ngterm.android.e2e

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.logs.LogsViewModel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
 * The diagnostic log of a troubleshooting session, end to end, and what it must never hold. A person signs in to a
 * real machine, adds a second one nobody answers at, opens a named session, types at its shell on the soft keyboard
 * (the test APK's own, [QaKeyboards]) and on a hardware one, pastes a line, and says no to a paste that could run a
 * command. Settings > Logs then tells what happened, newest first: the connect that failed and why, the attach, and
 * each change of what became of the input (sent, held as an unsafe paste). None of what was typed, pasted, held back
 * or printed by the shell, nor the session's name, the private key or a host key fingerprint, is in the lines shown,
 * in what Copy takes, in the file Share writes or in the log's files on the device. What is typed reaches the app a key
 * at a time, so the log is also held to what a key at a time could put there: it gains fewer lines over all the typing
 * than the shortest typed word has letters, and a line about input holds how it was delivered, its kind and its pane,
 * nothing else. Clear logs asks first: Cancel keeps every line, Clear logs deletes them and the log starts again.
 *
 * The stack is [SoftKeyboardSwitchE2eTest]'s: `scripts/test-android-e2e.sh` starts the machine and hands it over as
 * instrumentation arguments; without them the test is skipped.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.P)
class AppLogsPrivacyE2eTest {
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

    /** A fresh install (no key, no machine, no pin, no log, no exported log), cleared before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
                exportDir().deleteRecursively()
            }
        }

    private val keyboards = QaKeyboards(instrumentation)

    /** How many lines the log had as the first key was about to be typed. */
    private var linesBeforeTyping = 0

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                screenshot("failed")
                Log.e(TAG, "terminal at the failure:\n${terminalText()}")
            }
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstall)
            .around(keyboards)
            .around(compose)
            .around(evidence)

    @Test
    fun theLogTellsWhatHappenedAndHoldsNothingTypedPastedOrPrinted() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        signIn(key)
        addMachine(MACHINE, arguments.getString(PORT).orEmpty())
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
        // a machine nobody answers at: its card says so
        addMachine(REFUSED_MACHINE, REFUSED_PORT)
        awaitText(REFUSED_MACHINE)
        val unreachable = text(R.string.machine_unreachable)
        compose.waitUntil(CONNECT_WAIT_MS) {
            nodes(hasText(unreachable, substring = true) or hasContentDescription(unreachable, substring = true)) > 0
        }
        screenshot("unreachable")

        openANewSession()
        typeAtTheShell()
        pasteIntoTheShell()
        assertNoLinePerKey()
        screenshot("terminal")

        openLogs()
        for (line in TOLD) awaitText(line, substring = true)
        screenshot("shown")
        val shown = logText()
        assertNewestFirst(shown)
        assertClean("the lines shown", shown, key)

        compose.onNodeWithText(text(R.string.logs_copy)).performClick()
        awaitText(text(R.string.action_copied))
        val copied = copiedText()
        for (line in TOLD) assertTrue("the copied lines hold \"$line\":\n$copied", line in copied)
        assertClean("the copied lines", copied, key)

        compose.onNodeWithText(text(R.string.logs_share)).performClick()
        compose.waitUntil(SHEET_WAIT_MS) { exported() != null }
        compose.waitForIdle()
        val app = By.pkg(instrumentation.targetContext.packageName).depth(0)
        assertEquals("the share sheet opened", true, device.wait(Until.gone(app), SHEET_WAIT_MS))
        val whole = requireNotNull(exported()).readText()
        for (line in TOLD) assertTrue("the exported log holds \"$line\":\n$whole", line in whole)
        assertClean("the exported log", whole, key)
        assertInputLinesHoldNoText("the exported log", whole)
        assertClean("the log's files", logFiles(), key)
        assertInputLinesHoldNoText("the log's files", logFiles())
        device.pressBack()
        assertEquals("the app is back from the share sheet", true, device.wait(Until.hasObject(app), SHEET_WAIT_MS))

        clearAsksFirst()
        screenshot("cleared")
    }

    private fun signIn(key: String) {
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
    }

    private fun addMachine(
        label: String,
        port: String,
    ) {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(label)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(port)
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
    }

    private fun openANewSession() {
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }
        // the first attach asks for notifications (Android 13 and later): Not now, as one may answer
        val title = hasText(text(R.string.notifications_ask_title))
        val asked = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(title) > 0 } }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
    }

    // a sum on the soft keyboard, whose answer only the shell prints, then a word on a hardware keyboard
    private fun typeAtTheShell() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        compose.waitForIdle()
        val bounds = compose.onNode(terminal()).fetchSemanticsNode().boundsInWindow
        device.click(bounds.center.x.toInt(), bounds.center.y.toInt())
        keyboards.awaitShown(compose::waitForIdle)
        linesBeforeTyping = logLines().size
        TYPED.split(' ').forEach { word ->
            keyboards.tapKeys(word)
            keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.SPACE))
        }
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        awaitTerminal("the shell's answer to the sum") { text -> text.lines().any { it.trim() == PRINTED } }
        shell("input text $HARDWARE_TYPED")
        awaitTerminal("the word typed on a hardware keyboard") { HARDWARE_TYPED in it }
    }

    // a line pasted from the selection toolbar; then, in a program that takes a pasted line break as Enter (`cat`,
    // which the shell's own prompt does not), a paste of two lines, which asks first: Cancel
    private fun pasteIntoTheShell() {
        copy(PASTED)
        compose.onNode(terminal()).performTouchInput { longClick() }
        awaitText(text(R.string.terminal_paste))
        compose.onNodeWithText(text(R.string.terminal_paste)).performClick()
        awaitTerminal("the pasted line") { PASTED in it }
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        keyboards.tapKeys("cat")
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        // cat is reading once it says a line back: the line shows twice, as typed and as cat wrote it
        keyboards.tapKeys(ECHOED)
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        awaitTerminal("cat's copy of a typed line") { text -> text.lines().count { it.trim() == ECHOED } >= 2 }
        copy("$HELD_PASTE\n$HELD_PASTE")
        compose.onNode(terminal()).performTouchInput { longClick() }
        awaitText(text(R.string.terminal_paste))
        compose.onNodeWithText(text(R.string.terminal_paste)).performClick()
        awaitText(text(R.string.paste_unsafe_title))
        compose.onNodeWithText(text(R.string.action_cancel)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text(R.string.paste_unsafe_title))) == 0 }
        assertFalse("a paste that was cancelled reached the program:\n${terminalText()}", HELD_PASTE in terminalText())
    }

    private fun openLogs() {
        // a phone's session bar has Back, to the attached sessions' list, which has the navigation
        val back = hasContentDescription(text(R.string.action_back))
        if (nodes(back) > 0) compose.onNode(back).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text(R.string.nav_settings))) > 0 }
        compose.onAllNodesWithText(text(R.string.nav_settings))[0].performClick()
        awaitText(text(R.string.settings_logs))
        compose.onNodeWithText(text(R.string.settings_logs)).performScrollTo().performClick()
        awaitText(text(R.string.logs_never))
    }

    private fun clearAsksFirst() {
        val inDialog = hasText(text(R.string.logs_clear)) and hasAnyAncestor(isDialog())
        compose.onNodeWithText(text(R.string.logs_clear)).performClick()
        awaitText(text(R.string.logs_clear_title))
        compose.onNodeWithText(text(R.string.action_cancel)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text(R.string.logs_clear_title))) == 0 }
        assertTrue("Cancel kept the log's lines:\n${logText()}", TOLD.all { it in logText() })

        compose.onNodeWithText(text(R.string.logs_clear)).performClick()
        awaitText(text(R.string.logs_clear_title))
        compose.onNode(inDialog).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { CLEARED in logText() && TOLD.none { it in logText() } }
        val files = logFiles()
        assertTrue("the cleared log starts again with \"$CLEARED\":\n$files", CLEARED in files)
        assertTrue("the cleared log's files keep lines from before:\n$files", TOLD.none { it in files })
    }

    // the last thing done (going to Settings) is above the first (the connect)
    private fun assertNewestFirst(shown: String) {
        val newest = shown.indexOf(NEWEST)
        val oldest = shown.lastIndexOf(OLDEST)
        assertTrue("the log shows \"$NEWEST\" and \"$OLDEST\":\n$shown", newest >= 0 && oldest >= 0)
        assertTrue("the lines are newest first:\n$shown", newest < oldest)
    }

    // nothing typed, pasted, held back or printed, no session name, no key body, no host key fingerprint
    private fun assertClean(
        what: String,
        log: String,
        key: String,
    ) {
        for ((name, secret) in SECRETS) assertFalse("$what hold $name ($secret):\n$log", secret in log)
        for (line in key.lines().filter { it.length > KEY_LINE }) {
            assertFalse("$what hold key material", line in log)
        }
        assertFalse("$what hold a fingerprint", "SHA256:" in log)
    }

    // a key is typed alone, so a log that took each one would hold no typed word whole: it would gain a line a key
    private fun assertNoLinePerKey() {
        val gained = logLines().drop(linesBeforeTyping)
        assertTrue(
            "the log gained ${gained.size} lines while the keys were typed, as many as " +
                "\"${TYPED_MARKERS.minBy { it.length }}\" has letters or more:\n${gained.joinToString("\n")}",
            gained.size < TYPED_MARKERS.minOf { it.length },
        )
    }

    // a line about input: how it was delivered, its kind and its pane; one for each change, not one for each key
    private fun assertInputLinesHoldNoText(
        what: String,
        log: String,
    ) {
        val input = log.lines().filter { INPUT_TAG in it }
        assertTrue("$what tell of the input:\n$log", input.isNotEmpty())
        for (line in input) {
            assertTrue("$what hold a line about input with more than its delivery:\n$line", INPUT_LINE.matches(line))
        }
        assertTrue(
            "$what hold ${input.size} lines about input for $INPUT_CHANGES changes of its delivery:\n" +
                input.joinToString("\n"),
            input.size <= INPUT_CHANGES,
        )
    }

    private fun copy(text: String) {
        val clipboard = instrumentation.targetContext.getSystemService(ClipboardManager::class.java)
        instrumentation.runOnMainSync { clipboard.setPrimaryClip(ClipData.newPlainText("qa", text)) }
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

    // the log's lines as the screen shows them: the one text that holds lines' times
    private fun logText(): String =
        runCatching {
            compose
                .onAllNodes(
                    SemanticsMatcher("shows log lines") { node ->
                        node.config
                            .getOrNull(SemanticsProperties.Text)
                            .orEmpty()
                            .any { LINE.containsMatchIn(it.text) }
                    },
                ).fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    // the log as it is on the device: its file and the one it rotated to
    private fun logFiles(): String =
        File(instrumentation.targetContext.noBackupFilesDir, LOG_DIR)
            .walk()
            .filter { it.isFile }
            .joinToString("\n") { it.readText() }

    private fun logLines(): List<String> = logFiles().lines().filter { it.isNotBlank() }

    private fun exportDir() = File(instrumentation.targetContext.cacheDir, LogsViewModel.EXPORT_DIR)

    private fun exported() = exportDir().walk().firstOrNull { it.name == LogsViewModel.EXPORT_NAME }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    private fun terminalText(): String =
        runCatching {
            compose
                .onAllNodes(terminal())
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    private fun awaitTerminal(
        what: String,
        shows: (String) -> Boolean,
    ) {
        val came = runCatching { compose.waitUntil(ANSWER_WAIT_MS) { shows(terminalText()) } }.isSuccess
        assertTrue("the terminal never showed $what; it shows:\n${terminalText()}", came)
    }

    private fun awaitText(
        text: String,
        substring: Boolean = false,
    ) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text, substring = substring)) > 0 }

    private fun text(id: Int): String = instrumentation.targetContext.getString(id)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/app-logs-privacy-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"
        const val MACHINE = "qa-machine"
        const val REFUSED_MACHINE = "qa-nobody-home"

        /** A port of the device's own loopback that nothing listens on. */
        const val REFUSED_PORT = "9"

        const val SESSION = "qa-sess-mk5n"

        /** Typed on the soft keyboard; the shell prints [PRINTED], which is never typed. */
        const val SOFT_TYPED = "8641975300"
        const val TYPED = "expr $SOFT_TYPED + 12345"
        const val PRINTED = "8641987645"
        const val HARDWARE_TYPED = "qa-hw-mk2j"
        const val PASTED = "qa-paste-mk8v"
        const val HELD_PASTE = "qa-held-mk4w"
        const val ECHOED = "qa-echo-mk1c"

        /** What is typed a key at a time: on the soft keyboard, on a hardware one, into a program. */
        val TYPED_MARKERS = listOf(SOFT_TYPED, HARDWARE_TYPED, ECHOED)

        /** The log's lines about input: their tag, and the whole of one as the file holds it. */
        const val INPUT_TAG = " input: "
        val INPUT_LINE =
            Regex("""\S+ [IWE] input: input delivery=[a-z-]+ kind=(key|text|paste) pane=[0-9A-Fa-f-]{36}""")

        /** The input starts to go and a paste is held back; room for one drop and its end, far under a line a key. */
        const val INPUT_CHANGES = 4

        /** What must be in no form of the log. */
        val SECRETS =
            mapOf(
                "what was typed on the soft keyboard" to SOFT_TYPED,
                "what the shell printed" to PRINTED,
                "what was typed on a hardware keyboard" to HARDWARE_TYPED,
                "what was pasted" to PASTED,
                "a paste that was held back" to HELD_PASTE,
                "what was typed into a program" to ECHOED,
                "the session's name" to SESSION,
            )

        /** What the log tells of the session: the failed connect, the attach, and what became of the input. */
        val TOLD =
            listOf(
                "label=$MACHINE",
                "machine connected",
                "label=$REFUSED_MACHINE",
                "tcp connect failed",
                "connect failed",
                "listing answered",
                "new session",
                "attach asked",
                "attached",
                "delivery=sent",
                "kind=paste",
                "delivery=held-unsafe-paste",
                "destination=settings",
            )

        const val NEWEST = "destination=settings"
        const val OLDEST = "label=$MACHINE"
        const val CLEARED = "log cleared"

        /** A line as the screen shows it: its time, then its component. */
        val LINE = Regex("""(?m)^(\d{2}:\d{2}:\d{2}) [a-z-]+ """)
        const val LOG_DIR = "logs"
        const val KEY_LINE = 16
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val ANSWER_WAIT_MS = 15_000L
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
        const val SHEET_WAIT_MS = 15_000L
    }
}
