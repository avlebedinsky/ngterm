package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnySibling
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
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
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
import kotlin.math.roundToInt

/**
 * A chat message that can't be sent, end to end, as a person meets it: Claude Code runs in a named session, and its
 * prompt in the terminal already holds a few characters someone typed there and never entered. A long first message
 * sent from the chat is then not sent (it would go along with those characters). The chat says so, with the message
 * still in its field and Show terminal as the way on, and never the old "Use the terminal" with Answer in terminal.
 * Nothing was said yet, so the chat still says so over the banner, with the soft keyboard open: its words are there in
 * full, over the banner and not under it, and the banner's Show terminal is the one on screen, in full too.
 *
 * The message stays through every way to the terminal and back: the banner's Show terminal, then the session's More
 * menu (Chat view reads as off over the terminal picked, one tap brings the chat back); the bar's Show terminal, then
 * its Show chat. The terminal shows that nothing of it was sent. Once the prompt is cleared there, the same message
 * goes from the field it stayed in, and the agent answers it.
 *
 * The agent and the machine are [ChatFirstMessageE2eTest]'s: the stand-in Claude Code (`e2e-claude`) in a private
 * `ngterm-headless` behind the private `sshd` of `scripts/test-android-e2e.sh`; without the script's arguments the
 * test is skipped. No live ngterm, and no real agent.
 */
@RunWith(AndroidJUnit4::class)
class ChatUnsentMessageE2eTest {
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
    fun aMessageThePromptCannotTakeStaysInTheFieldThroughTheTerminalAndIsSentOnceThePromptIsClear() {
        signIn()
        addMachine()
        openANewSession()
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need; the test rule
        // runs what the tap starts only when the test asks
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        type(AGENT_COMMAND)
        shell("input keyevent KEYCODE_ENTER")
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }

        // a few characters typed into Claude Code's prompt at the terminal, and left there
        compose.onNode(barSwitch(R.string.session_show_terminal)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }
        compose.onNode(terminal()).performClick()
        type(LEFTOVER)
        compose.waitUntil(CONNECT_WAIT_MS) { LEFTOVER in terminalText() }
        compose.onNode(barSwitch(R.string.session_show_chat)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }

        // the first message, from the chat: not sent, and the chat says where it is
        compose.onNode(composer()).performTextInput(MESSAGE)
        compose.onNode(hasContentDescription(text(R.string.chat_send))).performClick()
        val told =
            runCatching { compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(notSent(), substring = true)) > 0 } }
                .isSuccess
        screenshot("not-sent")
        assertTrue("no banner says the message was not sent; the pane shows:\n${paneText()}", told)
        assertEquals("the field lost the message that was not sent", 1, nodes(composer() and hasText(MESSAGE)))
        assertEquals("the chat shows a message that was not sent", 0, nodes(hasContentDescription(said(MESSAGE))))
        assertEquals(0, nodes(hasText(text(R.string.chat_answer_in_terminal))))

        assertNothingSaidShowsInFullOverTheBanner()

        // the banner's way on, the terminal: nothing of the message went into it
        compose.onNode(bannerAction(R.string.session_show_terminal)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 && LEFTOVER in terminalText() }
        screenshot("terminal")
        assertFalse("the terminal got the message:\n${terminalText()}", MESSAGE_LAST_LINE in terminalWords())

        // back through More > Chat view, off over the terminal picked: the message is in the field
        compose.onNode(more()).performClick()
        awaitText(text(R.string.session_chat_view))
        assertEquals(ToggleableState.Off, chatViewState())
        compose.onNodeWithText(text(R.string.session_chat_view)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer() and hasText(MESSAGE)) > 0 }
        screenshot("back")

        // the prompt cleared at the terminal, then back through the bar: the same message goes, and is answered
        compose.onNode(barSwitch(R.string.session_show_terminal)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }
        compose.onNode(terminal()).performClick()
        shell("input keyevent" + " KEYCODE_DEL".repeat(LEFTOVER.length))
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 && LEFTOVER !in terminalText() }
        compose.onNode(barSwitch(R.string.session_show_chat)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer() and hasText(MESSAGE)) > 0 }
        compose.onNode(hasContentDescription(text(R.string.chat_send))).performClick()
        val sent =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasContentDescription(said(MESSAGE))) > 0 }
            }.isSuccess
        screenshot("sent")
        assertTrue("the message never showed in the chat; the pane shows:\n${paneText()}", sent)
        awaitText(REPLY)
        assertEquals("the field kept a message that was sent", 0, nodes(composer() and hasText(MESSAGE)))
        assertEquals("a banner stays over a message that was sent", 0, nodes(hasText(notSent(), substring = true)))
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

    // types [word] into the terminal, as a hardware keyboard does
    private fun type(word: String) {
        shell("input text $word")
    }

    // what the banner of a message that was not sent says, after the agent's name
    private fun notSent(): String = text(R.string.chat_problem_prompt_not_found, NAME_MARK).substringAfter(NAME_MARK)

    // the chat with nothing said, over the banner and the keyboard: its words in full, and one Show terminal
    private fun assertNothingSaidShowsInFullOverTheBanner() {
        assertTrue("the soft keyboard is not open", keyboardOpen())
        val banner = bounds(hasText(notSent(), substring = true))
        val emptyChat =
            listOf(
                hasText(text(R.string.chat_empty_title)),
                hasText(text(R.string.chat_empty_body, NAME_MARK).substringAfter(NAME_MARK), substring = true),
            )
        for (part in emptyChat) {
            assertEquals("the chat with nothing said lost ${part.description}", 1, nodes(part))
            assertTrue("the chat with nothing said shows ${part.description} cut", shownInFull(part))
            assertTrue("${part.description} lies under the banner", bounds(part).bottom <= banner.top)
        }
        assertEquals("the banner has no Show terminal", 1, nodes(bannerAction(R.string.session_show_terminal)))
        assertTrue("the banner's Show terminal is cut", shownInFull(bannerAction(R.string.session_show_terminal)))
        assertEquals("Show terminal is offered twice", 1, nodes(hasText(text(R.string.session_show_terminal))))
    }

    // the button beside the banner's message, not the one a chat with nothing said has under its own words
    private fun bannerAction(label: Int): SemanticsMatcher =
        hasText(text(label)) and hasAnySibling(hasText(notSent(), substring = true))

    private fun bounds(matcher: SemanticsMatcher): Rect = compose.onNode(matcher).fetchSemanticsNode().boundsInRoot

    // nothing of the node is clipped away: what is on screen is all of it
    private fun shownInFull(matcher: SemanticsMatcher): Boolean {
        val node = compose.onNode(matcher).fetchSemanticsNode()
        val shown = node.boundsInRoot
        return shown.width.roundToInt() == node.size.width && shown.height.roundToInt() == node.size.height
    }

    private fun keyboardOpen(): Boolean =
        compose.runOnUiThread {
            ViewCompat
                .getRootWindowInsets(compose.activity.window.decorView)
                ?.isVisible(WindowInsetsCompat.Type.ime()) == true
        }

    private fun chatViewState(): ToggleableState? =
        compose
            .onNode(hasText(text(R.string.session_chat_view)) and SemanticsMatcher.keyIsDefined(TOGGLE))
            .fetchSemanticsNode()
            .config
            .getOrNull(TOGGLE)

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun composer(): SemanticsMatcher = hasSetTextAction()

    // the session bar's switch to the focused pane's other view, named for what a tap does
    private fun barSwitch(label: Int): SemanticsMatcher = hasContentDescription(text(label))

    private fun more(): SemanticsMatcher = hasContentDescription(text(R.string.action_more, SESSION))

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

    // the terminal's text with its rows joined, so a message reads the same however the agent wrapped it
    private fun terminalWords(): String = terminalText().split(Regex("\\s+")).joinToString(" ")

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
        shell("screencap -p $EVIDENCE_DIR/chat-unsent-message-$name.png")
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

        const val MACHINE = "qa-unsent"
        const val SESSION = "qa-unsent-work"

        /** The stand-in Claude Code the script puts on the machine's PATH; it starts a session named `team-lead`. */
        const val AGENT_COMMAND = "e2e-claude"

        /** What the stand-in answers every prompt with. */
        const val REPLY = "ok"

        /** What someone typed into the agent's prompt at the terminal and never entered. */
        const val LEFTOVER = "leftover"

        const val MESSAGE_LAST_LINE = "Tell me what you found before you change anything."

        /** Long enough, and of enough lines, that Claude Code's prompt shows it as a pasted block. */
        const val MESSAGE =
            "The sessions list on my phone shows every session of the Mac, but a session that runs an agent " +
                "opens as a terminal and never as a chat.\n\n" +
                "What I expected:\n" +
                "- the chat view for a session with Claude Code\n" +
                "- the same for Codex\n\n" +
                "Check why this happens and whether the last release changed it.\n" +
                MESSAGE_LAST_LINE

        /** Stands in for the agent's name where a string resource takes it. */
        const val NAME_MARK = "\u0001"

        val TOGGLE = SemanticsProperties.ToggleableState

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
