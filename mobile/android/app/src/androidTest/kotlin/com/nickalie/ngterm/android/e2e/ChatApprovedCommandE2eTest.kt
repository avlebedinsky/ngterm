package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasClickAction
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
 * An approval answered from the chat for a command that runs long, end to end, as a person meets it: Claude Code, in
 * a named session, asks to run a shell command; its card is up and the session bar says Waiting. Yes on the card: the
 * bar says Working and the chat's foot "Claude Code is working" with Stop, at once and for as long as the command
 * runs, never Waiting or "Still running in the background". The chat opened again meanwhile shows no card. Stop: the
 * terminal shows the call Interrupted. The same ask answered No: the bar leaves Waiting and never says Working, and
 * the foot shows nothing working.
 *
 * Claude Code writes no status while an approved call runs (no hook fires until it ends), so what the bar and the foot
 * say comes from the origin hearing of the answer from the phone (`ngtermctl agent answered`).
 *
 * The agent and the machine are [ChatFirstMessageE2eTest]'s: the stand-in Claude Code (`e2e-claude`), whose dialog,
 * running turn and interrupt the mobile e2e's `fake_agent.py --standalone` draws, in a private `ngterm-headless`
 * behind the private `sshd` of `scripts/test-android-e2e.sh`; without the script's arguments the test is skipped. No
 * live ngterm, and no real agent.
 */
@RunWith(AndroidJUnit4::class)
class ChatApprovedCommandE2eTest {
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
                Log.e(TAG, "the pane at the failure:\n${paneText()}")
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
    fun yesShowsWorkingAndStopWhileTheApprovedCommandRunsAndNoLeavesNothingWorking() {
        signIn()
        addMachine()
        openANewSession()
        startTheAgent()

        // Yes: Working and Stop at once, and for the whole run
        ask("yes")
        val answered = SystemClock.elapsedRealtime()
        compose.onNode(option(YES)).performScrollTo().performClick()
        val shown = runCatching { compose.waitUntil(ANSWER_WAIT_MS) { working() } }.isSuccess
        val after = SystemClock.elapsedRealtime() - answered
        Log.i(TAG, "Working and Stop shown ${after}ms after Yes")
        screenshot("yes-working")
        assertTrue("no Working with Stop ${ANSWER_WAIT_MS}ms after Yes; the pane shows:\n${paneText()}", shown)
        holdWorking("yes-held")

        // the chat opened again while the command runs: it loads with Working and Stop, and no card comes back
        compose.onNode(barSwitch(R.string.session_show_terminal)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }
        compose.onNode(barSwitch(R.string.session_show_chat)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }
        val reopened = SystemClock.elapsedRealtime() + ANSWER_WAIT_MS
        while (!working() && SystemClock.elapsedRealtime() < reopened) {
            assertEquals("the answered card came back in the chat opened again", 0, nodes(card()))
            assertEquals("the session waits again", 0, nodes(badge(R.string.agent_blocked)))
            SystemClock.sleep(SAMPLE_MS)
        }
        holdWorking("reopened")

        // Stop: the call is interrupted
        compose.onNode(stop()).performClick()
        compose.onNode(barSwitch(R.string.session_show_terminal)).performClick()
        val interrupted =
            runCatching { compose.waitUntil(CONNECT_WAIT_MS) { INTERRUPTED in terminalText() } }.isSuccess
        screenshot("stopped")
        assertTrue("the terminal never showed the call interrupted:\n${terminalText()}", interrupted)
        compose.onNode(barSwitch(R.string.session_show_chat)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }

        compose.waitUntil(ANSWER_WAIT_MS) {
            nodes(badge(R.string.agent_active)) == 0 && nodes(stop()) == 0
        }

        // No: the session leaves Waiting and never shows Working
        ask("no")
        compose.onNode(option(NO)).performScrollTo().performClick()
        val cleared =
            runCatching {
                compose.waitUntil(ANSWER_WAIT_MS) { nodes(card()) == 0 && nodes(badge(R.string.agent_blocked)) == 0 }
            }.isSuccess
        screenshot("no")
        assertTrue("the session still waits ${ANSWER_WAIT_MS}ms after No; the pane shows:\n${paneText()}", cleared)
        val until = SystemClock.elapsedRealtime() + NO_HOLD_MS
        while (SystemClock.elapsedRealtime() < until) {
            assertEquals("Working after No; the pane shows:\n${paneText()}", 0, nodes(badge(R.string.agent_active)))
            assertEquals("the foot shows a turn working after No", 0, nodes(stop()))
            assertEquals("the foot shows a turn working after No", 0, nodes(hasText(workingLine(), substring = true)))
            SystemClock.sleep(SAMPLE_MS)
        }
        compose.onNode(barSwitch(R.string.session_show_terminal)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().split(INTERRUPTED).size > 2 }
        screenshot("no-terminal")
    }

    // the prompt that makes the agent ask to run COMMAND, sent from the chat, and its card up with the session waiting
    private fun ask(step: String) {
        compose.onNode(composer()).performTextInput(PROMPT)
        compose.onNode(hasContentDescription(text(R.string.chat_send))).performClick()
        val asked =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) {
                    nodes(option(YES)) > 0 && nodes(badge(R.string.agent_blocked)) > 0
                }
            }.isSuccess
        screenshot("$step-card")
        assertTrue("no card with the session waiting; the pane shows:\n${paneText()}", asked)
        assertTrue("the card names another call", nodes(hasText(COMMAND)) > 0)
    }

    // the bar says Working and the foot "Claude Code is working" with Stop, with no card, through HOLD_MS
    private fun holdWorking(step: String) {
        val until = SystemClock.elapsedRealtime() + HOLD_MS
        while (SystemClock.elapsedRealtime() < until) {
            if (!working()) {
                throw AssertionError("the session stopped showing Working with Stop (${missing()}):\n${paneText()}")
            }
            assertEquals("the session waits again", 0, nodes(badge(R.string.agent_blocked)))
            assertEquals("the answered card came back", 0, nodes(card()))
            assertEquals(0, nodes(hasText(text(R.string.chat_activity_background), substring = true)))
            SystemClock.sleep(SAMPLE_MS)
        }
        screenshot(step)
    }

    private fun working(): Boolean =
        nodes(badge(R.string.agent_active)) > 0 &&
            nodes(hasText(workingLine(), substring = true)) > 0 &&
            nodes(stop()) > 0 &&
            nodes(card()) == 0

    // what of working() is not on screen
    private fun missing(): String =
        listOfNotNull(
            "the bar's Working".takeIf { nodes(badge(R.string.agent_active)) == 0 },
            "the foot's working line".takeIf { nodes(hasText(workingLine(), substring = true)) == 0 },
            "Stop".takeIf { nodes(stop()) == 0 },
            "no card".takeIf { nodes(card()) > 0 },
        ).joinToString()

    private fun workingLine(): String = text(R.string.chat_activity_working, text(R.string.chat_agent_claude))

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

    // the stand-in Claude Code, started in the session's shell: the pane turns into its chat
    private fun startTheAgent() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need; the test rule
        // runs what the tap starts only when the test asks
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        type(AGENT_COMMAND)
        shell("input keyevent KEYCODE_ENTER")
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }
    }

    // types [word] into the terminal, as a hardware keyboard does
    private fun type(word: String) {
        shell("input text $word")
    }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun composer(): SemanticsMatcher = hasSetTextAction()

    // the approval card, by its title
    private fun card(): SemanticsMatcher = hasText(text(R.string.chat_approval_title, TOOL))

    // one of the card's options; with the keyboard up the card scrolls, and a later option lies under its fold
    private fun option(label: String): SemanticsMatcher = hasText(label) and hasClickAction()

    // the foot's Stop
    private fun stop(): SemanticsMatcher = hasText(text(R.string.chat_stop)) and hasClickAction()

    // the session bar's status, as TalkBack reads it
    private fun badge(word: Int): SemanticsMatcher = hasContentDescription(text(word))

    // the session bar's switch to the focused pane's other view, named for what a tap does
    private fun barSwitch(label: Int): SemanticsMatcher = hasContentDescription(text(label))

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

    // what the pane shows, in either view: every text and spoken label on screen
    private fun paneText(): String =
        runCatching {
            compose
                .onAllNodes(
                    SemanticsMatcher("has text or a label") { node ->
                        node.config.contains(SemanticsProperties.Text) ||
                            node.config.contains(SemanticsProperties.ContentDescription)
                    },
                ).fetchSemanticsNodes()
                .flatMap { node ->
                    node.config
                        .getOrNull(SemanticsProperties.Text)
                        .orEmpty()
                        .map { it.text } +
                        node.config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()
                }.joinToString("\n")
        }.getOrDefault("")

    private fun awaitText(text: String) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/chat-approved-command-$name.png")
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

        const val MACHINE = "qa-approved"
        const val SESSION = "qa-approved-work"

        /** The stand-in Claude Code the script puts on the machine's PATH; it starts a session named `team-lead`. */
        const val AGENT_COMMAND = "e2e-claude"

        /** A command that runs long once approved, and the prompt that has the agent ask to run it. */
        const val COMMAND = "python3 -c \"import time; time.sleep(90)\""
        const val PROMPT = "Run this shell command with the Bash tool: $COMMAND"
        const val TOOL = "Bash"

        /** The card's options, as Claude Code's dialog words them. */
        const val YES = "Yes"
        const val NO = "No"

        /** What Claude Code shows under a call refused or interrupted. */
        const val INTERRUPTED = "Interrupted"

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L

        /** How soon after the answer the session shows what it led to. */
        const val ANSWER_WAIT_MS = 5_000L

        /** How long Working and Stop are held while the approved command runs, and nothing working after No. */
        const val HOLD_MS = 20_000L
        const val NO_HOLD_MS = 10_000L
        const val SAMPLE_MS = 500L
    }
}
