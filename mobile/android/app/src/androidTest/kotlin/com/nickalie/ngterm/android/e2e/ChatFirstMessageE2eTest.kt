package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
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
 * The first message of a chat, end to end, as a person sends it: a new session on an ngterm machine, Claude Code
 * started in its shell under a name (which Claude Code draws into the rule over its prompt), the pane turning into its
 * chat with nothing said yet, and a long message of several paragraphs sent from the composer. The message reaches the
 * agent (its reply shows), shows in the chat as it was written, with none of the markup Claude Code keeps a long
 * paste in, and no banner sends the user to the terminal; the terminal, picked from the bar, shows the message too.
 *
 * A message still being written survives the trip to the terminal and back, and the way back is the session's More
 * menu: its Chat view reads as off while the pane shows the terminal picked for it, and one tap on it brings the chat
 * back with the text in its field.
 *
 * The agent is the mobile e2e's stand-in Claude Code in its standalone form (`e2e-claude` on the machine's PATH:
 * `crates/ngterm-mobile/tests/support/fake_agent.py --standalone`), which draws Claude Code 2.1.325's prompt of a
 * named session at the pane's grid and writes its transcript as Claude Code does. The machine is the private
 * `ngterm-headless` behind the private `sshd` that `scripts/test-android-e2e.sh` starts; without the script's
 * arguments the test is skipped. No live ngterm, and no real agent.
 */
@RunWith(AndroidJUnit4::class)
class ChatFirstMessageE2eTest {
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
                Log.e(TAG, "terminal at the failure:\n${terminalText()}")
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
    fun aLongFirstMessageReachesANamedClaudeSessionAndADraftSurvivesTheTerminal() {
        signIn()
        addMachine()
        openANewSession()

        // Claude Code, started in the session's shell as a person starts it: the pane turns into its chat
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need; the test rule
        // runs what the tap starts only when the test asks
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        type(AGENT_COMMAND)
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(composer()) > 0 }
        awaitText(text(R.string.chat_empty_title))
        screenshot("opened")

        // the first message: long, several paragraphs
        compose.onNode(composer()).performTextInput(MESSAGE)
        compose.onNode(hasContentDescription(text(R.string.chat_send))).performClick()
        val sent =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasContentDescription(said(MESSAGE))) > 0 }
            }.isSuccess
        screenshot("sent")
        val banners =
            nodes(hasText(text(R.string.session_show_terminal))) +
                nodes(hasText(text(R.string.chat_answer_in_terminal)))
        assertEquals("a banner sends the user to the terminal; the pane shows:\n${paneText()}", 0, banners)
        assertTrue("the chat never showed the message as it was written; the pane shows:\n${paneText()}", sent)
        awaitText(REPLY)
        assertEquals("the field kept a message that was sent", 0, nodes(hasText(MESSAGE)))

        // a message being written, then the terminal: it shows the message that was sent
        compose.onNode(composer()).performTextInput(DRAFT)
        compose.onNode(hasContentDescription(text(R.string.session_show_terminal))).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 && MESSAGE_LAST_LINE in terminalWords() }
        screenshot("terminal")

        // the way back a person looks for: More > Chat view, which reads as off while the terminal is picked
        compose.onNode(more()).performClick()
        awaitText(text(R.string.session_chat_view))
        assertEquals(ToggleableState.Off, chatViewState())
        compose.onNodeWithText(text(R.string.session_chat_view)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasContentDescription(said(MESSAGE))) > 0 }
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(DRAFT)) > 0 }
        compose.onNode(more()).performClick()
        awaitText(text(R.string.session_chat_view))
        assertEquals(ToggleableState.On, chatViewState())
        screenshot("back")
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

    // types [command] and Enter into the terminal, as a hardware keyboard does
    private fun type(command: String) {
        shell("input text $command")
        shell("input keyevent KEYCODE_ENTER")
    }

    private fun chatViewState(): ToggleableState? =
        compose
            .onNode(hasText(text(R.string.session_chat_view)) and SemanticsMatcher.keyIsDefined(TOGGLE))
            .fetchSemanticsNode()
            .config
            .getOrNull(TOGGLE)

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun composer(): SemanticsMatcher = hasSetTextAction()

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
        shell("screencap -p $EVIDENCE_DIR/chat-first-message-$name.png")
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

        /** The stand-in Claude Code the script puts on the machine's PATH; it starts a session named `team-lead`. */
        const val AGENT_COMMAND = "e2e-claude"

        /** What the stand-in answers every prompt with. */
        const val REPLY = "ok"

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

        const val DRAFT = "one more thing, still being written"

        val TOGGLE = SemanticsProperties.ToggleableState

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
