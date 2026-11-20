package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
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
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
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
 * The diagnostic log of a chat, end to end: a person starts Claude Code in a new session's shell, sends a message from
 * the chat's composer (the agent answers it) and starts to write a second one. Settings > Logs then tells that the
 * message was sent and that the chat had a prompt of the user's, and shows the page's own line, the newest, as it
 * opens. Neither message's text is in the lines shown or in the log's files on the device.
 *
 * The agent is the mobile e2e's stand-in Claude Code in its standalone form (`e2e-claude` on the machine's PATH), as
 * in [ChatFirstMessageE2eTest]. The machine is the private `ngterm-headless` behind the private `sshd` that
 * `scripts/test-android-e2e.sh` starts; without the script's arguments the test is skipped. No live ngterm, and no
 * real agent.
 */
@RunWith(AndroidJUnit4::class)
class AppLogsChatE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
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

    /** A fresh install (no key, no machine, no pin), cleared before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                screenshot("failed")
                Log.e(TAG, "the screen at the failure:\n${screenText()}")
            }
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstall)
            .around(compose)
            .around(evidence)

    @Test
    fun theLogTellsOfASentMessageAndHoldsNoMessageText() {
        signIn()
        addMachine()
        openANewSession()
        startTheAgent()

        compose.onNode(composer()).performTextInput(MESSAGE)
        compose.onNode(hasContentDescription(text(R.string.chat_send))).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasContentDescription(said(MESSAGE))) > 0 }
        awaitText(REPLY)
        // a message still being written is in no line either
        compose.onNode(composer()).performTextInput(DRAFT)
        screenshot("sent")

        openLogs()
        // the page's own line, written as it opens, is the newest one it shows
        awaitText(OPENED, substring = true)
        for (line in TOLD) awaitText(line, substring = true)
        screenshot("shown")
        val shown = screenText()
        assertTrue("the newest line shown is the page's own:\n$shown", shown.indexOf(OPENED) < shown.indexOf(SENT))
        for (marker in MARKERS) {
            assertFalse("the lines shown hold a message's text ($marker):\n$shown", marker in shown)
            assertFalse("the log's files hold a message's text ($marker)", marker in logFiles())
        }
        val files = logFiles()
        for (line in TOLD) assertTrue("the log's files hold \"$line\":\n$files", line in files)
    }

    private fun signIn() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
    }

    private fun addMachine() {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(MACHINE)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
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

    // Claude Code, started in the session's shell as a person starts it: the pane turns into its chat
    private fun startTheAgent() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        shell("input text $AGENT_COMMAND")
        shell("input keyevent KEYCODE_ENTER")
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }
        awaitText(text(R.string.chat_empty_title))
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

    // the log as it is on the device: its file and the one it rotated to
    private fun logFiles(): String =
        File(instrumentation.targetContext.noBackupFilesDir, LOG_DIR)
            .walk()
            .filter { it.isFile }
            .joinToString("\n") { it.readText() }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun composer(): SemanticsMatcher = hasSetTextAction()

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    // the row of a message the user sent, as TalkBack reads it
    private fun said(message: String): String = text(R.string.chat_you_said, message)

    private fun terminalText(): String =
        runCatching {
            compose
                .onAllNodes(terminal())
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    // every text on screen, in the order of the screen
    private fun screenText(): String =
        runCatching {
            compose
                .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text))
                .fetchSemanticsNodes()
                .flatMap { node ->
                    node.config
                        .getOrNull(SemanticsProperties.Text)
                        .orEmpty()
                        .map { it.text }
                }.joinToString("\n")
        }.getOrDefault("")

    private fun awaitText(
        text: String,
        substring: Boolean = false,
    ) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text, substring = substring)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/app-logs-chat-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the host, the machine's port, the user and key. */
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-chat"
        const val SESSION = "qa-chat-work"

        /** The stand-in Claude Code the script puts on the machine's PATH. */
        const val AGENT_COMMAND = "e2e-claude"

        /** What the stand-in answers every prompt with. */
        const val REPLY = "ok"

        /** The text of the message sent and of the one still being written: each holds a word found nowhere else. */
        const val MESSAGE_MARKER = "qa-chat-mk6t"
        const val DRAFT_MARKER = "qa-chat2-mk3r"
        const val MESSAGE = "$MESSAGE_MARKER check the build"
        const val DRAFT = "$DRAFT_MARKER one more thing"
        val MARKERS = listOf(MESSAGE_MARKER, DRAFT_MARKER)

        /** The page's own line, which Settings writes as the page opens. */
        const val OPENED = "settings page=logs"

        /** The app's line for a message the agent got. */
        const val SENT = "action result=sent"

        /** What the log tells of the chat: the message went, and the chat had a prompt of the user's. */
        val TOLD = listOf(SENT, "prompts=1")

        const val LOG_DIR = "logs"
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
