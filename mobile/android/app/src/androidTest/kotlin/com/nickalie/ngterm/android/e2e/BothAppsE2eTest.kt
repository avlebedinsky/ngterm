package com.nickalie.ngterm.android.e2e

import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
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
import androidx.test.uiautomator.UiDevice
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
 * One machine that runs both apps, ngterm and upstream agterm, end to end: the app signs in to it once over ssh, and
 * its sessions screen lists the sessions of both. The machine is ngterm's (no badge on its card or its sessions
 * screen's top row); the agterm session's row alone carries the "agterm" badge, and the attached session's bar the
 * badge with its note. The agterm session attaches through agterm's bundled zmx (control taken first where agterm's own
 * window leads the pane) and what is typed on the soft keyboard reaches its shell; every menu of it has Chat view off
 * with "agterm has no chat view". The ngterm session beside it
 * is made from the same screen (New session goes to ngterm), takes typing, has no badge and keeps Chat view in every
 * menu. End session on the agterm session goes to agterm's CLI: it leaves every list, and the machine, whose agterm now
 * lists nothing, still lists its ngterm session, with no badge left.
 *
 * The machine is the private `sshd` of `scripts/test-android-e2e.sh` on its port for a host that runs both
 * ([BOTH_PORT]): its logins see the script's private `ngterm-headless` and, beside it, the conformance suite's stand-in
 * for agterm v0.41.1 (`conformance/agterm_conformance/fake_agterm.py`), seeded with the session [AGTERM_SESSION].
 * Without the script's arguments the test is skipped. No live agterm or ngterm. The keyboard is the test APK's own
 * ([QaKeyboards]), as in `SoftKeyboardSwitchE2eTest`.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.P)
class BothAppsE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine that runs both apps: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, BOTH_PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
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

    private val keyboards = QaKeyboards(instrumentation)

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
    fun aMachineRunningBothAppsListsTheSessionsOfBothAndEachWorksAsItsAppDoes() {
        signIn()
        addMachine()

        // one listing asked both apps: agterm's session is there before ngterm has any of this test's
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(AGTERM_SESSION)

        // New session on a machine that runs both goes to ngterm: no badge on its bar, Chat view in its menu, and
        // what is typed reaches its shell
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(NGTERM_SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        awaitTerminal()
        notNowToNotifications()
        assertEquals("the ngterm session's bar has an agterm badge", 0, badges())
        assertEveryMenuOffersChatView(NGTERM_SESSION)
        typeAndExpectTheAnswer(NGTERM_STEP, "the ngterm session")
        screenshot("ngterm-typed")
        toMachines()

        // the machine is ngterm's: its card has no badge
        assertEquals("the card of a machine that runs both has an agterm badge", 0, badges())
        screenshot("card")

        // its sessions screen lists both apps' sessions; the agterm one alone has the badge, in its own row
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(AGTERM_SESSION)
        awaitText(NGTERM_SESSION)
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(row(AGTERM_SESSION) and badge()) > 0 }
        assertEquals("the ngterm session's row has an agterm badge", 0, nodes(row(NGTERM_SESSION) and badge()))
        assertEquals("the machine's top row has its own agterm badge", 0, nodes(notedBadge()))
        screenshot("sessions")
        assertEveryMenuHasNoChatView(AGTERM_SESSION)
        assertEveryMenuOffersChatView(NGTERM_SESSION)

        // the agterm session attaches through agterm's zmx, and typing reaches its shell
        compose.onNodeWithText(AGTERM_SESSION).performClick()
        awaitTerminal()
        takeControlWhenOffered()
        val attached =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) { "attached agterm-" in terminalText() && PROMPT in terminalText() }
            }.isSuccess
        Log.i(TAG, "the agterm session's terminal:\n${terminalText()}")
        assertTrue("agterm's shell never showed in the pane; it shows:\n${terminalText()}", attached)
        typeAndExpectTheAnswer(AGTERM_STEP, "the agterm session")
        screenshot("agterm-typed")

        // its bar carries the badge with agterm's note, and its menu has no chat view
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(notedBadge()) > 0 }
        assertEveryMenuHasNoChatView(AGTERM_SESSION)
        screenshot("agterm-attached")

        // the attached list holds both: the agterm session's menu has no chat view, the ngterm one's keeps it
        toAttachedList()
        screenshot("attached-list")
        assertEveryMenuOffersChatView(NGTERM_SESSION)
        assertEveryMenuHasNoChatView(AGTERM_SESSION)

        // End session goes to agterm's CLI, and the session leaves every list
        compose.onAllNodes(more(AGTERM_SESSION))[0].performClick()
        compose.onNodeWithText(text(R.string.machine_sessions_end)).performClick()
        compose.onNode(hasText(text(R.string.machine_sessions_end)) and hasAnyAncestor(isDialog())).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(AGTERM_SESSION)) == 0 }
        awaitText(NGTERM_SESSION)

        // agterm lists nothing now: the machine still lists its ngterm session, with no badge anywhere
        toMachines()
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(NGTERM_SESSION)
        assertEquals("a session agterm ended is still listed", 0, nodes(hasText(AGTERM_SESSION)))
        assertEquals("an agterm badge stays with no agterm session listed", 0, badges())
        assertEveryMenuOffersChatView(NGTERM_SESSION)
        screenshot("agterm-lists-nothing")
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
        val port = arguments.getString(BOTH_PORT).orEmpty()
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(port)
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
    }

    /**
     * A tap on the terminal brings up the soft keyboard, the test's; then `expr 7000 + <step>` and Enter on it, and the
     * shell's answer on the terminal's screen, which the typed line itself never shows.
     */
    private fun typeAndExpectTheAnswer(
        step: Int,
        where: String,
    ) {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        compose.waitForIdle()
        val bounds = compose.onAllNodes(terminal())[0].fetchSemanticsNode().boundsInWindow
        device.click(bounds.center.x.toInt(), bounds.center.y.toInt())
        keyboards.awaitShown(compose::waitForIdle)
        listOf("expr", "$ANSWER_BASE", "+", "$step").forEach { word ->
            keyboards.tapKeys(word)
            keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.SPACE))
        }
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        val answer = "${ANSWER_BASE + step}"
        val came =
            runCatching {
                compose.waitUntil(ANSWER_WAIT_MS) { terminalText().lines().any { it.trim() == answer } }
            }.isSuccess
        Log.i(TAG, "$where: terminal now:\n${terminalText()}")
        assertTrue("$where: its shell never answered $answer; the terminal shows:\n${terminalText()}", came)
    }

    /** A pane agterm's own window leads is covered with Take control: taken, as one does to type there. */
    private fun takeControlWhenOffered() {
        val takeControl = hasText(text(R.string.session_take_control))
        val offered = runCatching { compose.waitUntil(COVER_WAIT_MS) { nodes(takeControl) > 0 } }.isSuccess
        if (!offered) return
        screenshot("agterm-covered")
        compose.onNode(takeControl).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(takeControl) == 0 }
    }

    /** Every More menu of [session] on screen: Chat view disabled, with "agterm has no chat view" under it. */
    private fun assertEveryMenuHasNoChatView(session: String) {
        val menus = nodes(more(session))
        assertTrue("no More menu of $session on screen", menus > 0)
        repeat(menus) { index ->
            compose.onAllNodes(more(session))[index].performClick()
            awaitText(text(R.string.session_chat_view_unavailable, AGTERM))
            compose.onNodeWithText(text(R.string.session_chat_view)).assertIsNotEnabled()
            closeTheMenu()
        }
    }

    /** Every More menu of [session] on screen: Chat view takes a tap, with no word of agterm under it. */
    private fun assertEveryMenuOffersChatView(session: String) {
        val menus = nodes(more(session))
        assertTrue("no More menu of $session on screen", menus > 0)
        repeat(menus) { index ->
            compose.onAllNodes(more(session))[index].performClick()
            awaitText(text(R.string.session_chat_view))
            compose.onNodeWithText(text(R.string.session_chat_view)).assertIsEnabled()
            assertEquals(
                "$session's menu says agterm has no chat view",
                0,
                nodes(hasText(text(R.string.session_chat_view_unavailable, AGTERM))),
            )
            closeTheMenu()
        }
    }

    private fun closeTheMenu() {
        device.pressBack()
        compose.waitUntil(MENU_WAIT_MS) { nodes(hasText(text(R.string.session_chat_view))) == 0 }
    }

    /**
     * Closes the open session to the attached list, in every layout: a phone's session bar has Back; a wider screen's
     * has none, so system Back closes it there (a first Back may only put the keyboard away).
     */
    private fun toAttachedList() {
        val back = hasContentDescription(text(R.string.action_back))
        if (nodes(back) > 0) compose.onAllNodes(back)[0].performClick()
        repeat(SYSTEM_BACKS) {
            if (nodes(terminal()) > 0) {
                device.pressBack()
                runCatching { compose.waitUntil(MENU_WAIT_MS) { nodes(terminal()) == 0 } }
            }
        }
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) == 0 && nodes(more(NGTERM_SESSION)) > 0 }
    }

    private fun toMachines() {
        val back = hasContentDescription(text(R.string.action_back))
        if (nodes(back) > 0) compose.onAllNodes(back)[0].performClick()
        compose.onNodeWithText(text(R.string.nav_machines)).performClick()
        awaitText(text(R.string.machines_add))
    }

    /** The first attach asks for notifications (Android 13 and later): Not now, as one may answer. */
    private fun notNowToNotifications() {
        val title = hasText(text(R.string.notifications_ask_title))
        val asked = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(title) > 0 } }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
    }

    private fun awaitTerminal() = compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    /** Every agterm badge on screen, a session row's plain one and the one with a note alike. */
    private fun badges(): Int =
        compose.onAllNodes(badge() or notedBadge(), useUnmergedTree = true).fetchSemanticsNodes().size

    // a session row's badge: the app's word alone, read with the row
    private fun badge(): SemanticsMatcher = hasContentDescription(AGTERM)

    // the badge that explains itself: its spoken label is "agterm. <the note>"
    private fun notedBadge(): SemanticsMatcher = hasContentDescription("$AGTERM. ", substring = true)

    private fun row(session: String): SemanticsMatcher = hasText(session)

    private fun more(session: String): SemanticsMatcher = hasContentDescription(text(R.string.action_more, session))

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

    private fun awaitText(text: String) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/both-apps-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the host, the both-apps machine's port, user and key. */
        const val HOST = "e2eHost"
        const val BOTH_PORT = "e2eBothPort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-both"
        const val NGTERM_SESSION = "qa-both-ngterm"

        /** The session the script seeds on the machine's stand-in agterm, and the prompt its zmx's shell shows. */
        const val AGTERM_SESSION = "qa-both-agterm"
        const val PROMPT = "agterm$"
        const val AGTERM = "agterm"

        const val ANSWER_BASE = 7000
        const val NGTERM_STEP = 1
        const val AGTERM_STEP = 2

        /** Where screenshots go on the device; scripts/test-android-e2e.sh collects them. */
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val ANSWER_WAIT_MS = 10_000L
        const val CONNECT_WAIT_MS = 60_000L
        const val MENU_WAIT_MS = 5_000L
        const val COVER_WAIT_MS = 8_000L
        const val ASK_WAIT_MS = 5_000L
        const val SYSTEM_BACKS = 2
    }
}
