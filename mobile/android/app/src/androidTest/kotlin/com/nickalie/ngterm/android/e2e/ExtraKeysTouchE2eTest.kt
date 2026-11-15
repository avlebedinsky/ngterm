package com.nickalie.ngterm.android.e2e

import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import android.view.InputDevice
import android.view.MotionEvent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
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
import org.junit.Assert.assertEquals
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
 * The extra-keys row under a real finger, end to end: in a live session on a real machine, a key goes to the shell
 * as the finger lifts inside it, once, and a touch that turns out to be something else sends nothing: one that slides
 * off its key, a swipe along the row, a back swipe from the screen's edge over the row. Ctrl and Alt arm, lock and let
 * go on taps and not on a touch that slides off. An arrow sends one key on a quick tap and on a short hold, repeats
 * while held and stops as the finger lifts.
 *
 * The real stack, as [SoftKeyboardSwitchE2eTest] has it (`scripts/test-android-e2e.sh`'s private `ngterm-headless`
 * behind a private `sshd`; without its arguments the test is skipped). The touches are the system's: down, move and
 * up events injected into the device's input pipeline as a touchscreen's, at the keys' places on the screen, so the
 * system's gesture detection sees them too. What the session received is read from the session itself: the shell
 * runs `cat -te`, which prints each line it got with its control characters spelled out (`^[` for Esc, `^I` for Tab)
 * and a `$` at its end; every step types a marker on the test's soft keyboard, touches the row, types `z` and Enter,
 * and compares the line `cat` printed. The steps run in one session and each failure is kept, so one failed step
 * does not hide the others.
 *
 * The device is put into gesture navigation for the test (and back after it), where a swipe from the screen's edge
 * is the system's back. A last step checks that the same swipe beside the row, over the terminal, is taken as back
 * (it hides the soft keyboard): the row's swipes were kept by the row, not missed by the system.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.Q)
class ExtraKeysTouchE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val testPackage = instrumentation.context.packageName
    private val compose = createAndroidComposeRule<MainActivity>()
    private val failures = mutableListOf<String>()

    /** The stack's address, or the skip, before anything starts. */
    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin), cleared before the activity starts, as LaunchSmokeTest does. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    /**
     * The test's keyboard on, the device's animations off (a key is touched where it rests) and gesture navigation
     * on for the test; the device's own input method, animations and navigation back after it, also when the setup
     * itself fails half way. Only what the test changed is put back, to the value it had.
     */
    private val deviceSetup =
        object : ExternalResource() {
            private val scales = mutableMapOf<String, String>()
            private var navigationBefore: String? = null
            private var keyboardBefore: String? = null
            private var keyboardChanged = false

            override fun before() {
                // JUnit runs after() only once before() has returned: a setup that fails puts the device back itself
                runCatching { setUp() }
                    .onFailure { runCatching { after() } }
                    .getOrThrow()
            }

            private fun setUp() {
                ANIMATION_SCALES.forEach { name ->
                    scales[name] = shell("settings get global $name").trim()
                    shell("settings put global $name 0")
                }
                val navigation = navigationMode()
                if (navigation != GESTURES) {
                    navigationBefore = navigation
                    shell("cmd overlay enable-exclusive --category $GESTURE_OVERLAY")
                }
                awaitShell("gesture navigation is on (now ${navigationMode()})") { navigationMode() == GESTURES }
                keyboardBefore = currentKeyboard().takeIf { it.contains('/') }
                keyboardChanged = true
                // a test APK installed a moment ago may not be listed as an input method yet: until it is, try again
                val keyboard = QaKeyboard.id(testPackage)
                awaitShell("the test keyboard is the device's input method (now ${currentKeyboard()})") {
                    if (currentKeyboard() != keyboard) {
                        shell("ime enable $keyboard")
                        shell("ime set $keyboard")
                    }
                    currentKeyboard() == keyboard
                }
            }

            private fun currentKeyboard() = shell("settings get secure default_input_method").trim()

            private fun navigationMode() = shell("settings get secure navigation_mode").trim()

            private fun awaitShell(
                what: String,
                met: () -> Boolean,
            ) {
                val deadline = SystemClock.uptimeMillis() + KEY_WAIT_MS
                var done = met()
                while (!done && SystemClock.uptimeMillis() < deadline) {
                    SystemClock.sleep(RETRY_MS)
                    done = met()
                }
                assertTrue(what, done)
            }

            override fun after() {
                if (keyboardChanged) {
                    keyboardBefore?.let { shell("ime set $it") } ?: shell("ime reset")
                    shell("ime disable ${QaKeyboard.id(testPackage)}")
                }
                navigationBefore?.let { mode ->
                    // the overlay of the mode the device was in; a mode with no overlay of its own: the test's one off
                    NAVIGATION_OVERLAYS[mode]?.let { shell("cmd overlay enable-exclusive --category $it") }
                        ?: shell("cmd overlay disable $GESTURE_OVERLAY")
                }
                scales.forEach { (name, value) ->
                    shell(if (value == "null") "settings delete global $name" else "settings put global $name $value")
                }
            }
        }

    /** A screenshot and the terminal's text as the test fails, kept where the script collects them. */
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
            .around(deviceSetup)
            .around(compose)
            .around(evidence)

    @Test
    fun aKeyGoesToTheShellAsTheFingerLiftsInsideItAndATouchThatIsSomethingElseSendsNothing() {
        signInAndOpenANewSession()
        showKeyboard()
        // the row stands aside for a hardware keyboard, which an emulator may have
        assertTrue(
            "the extra-keys row is not shown: the device has a hardware keyboard, or the row is hidden in Settings",
            compose.onAllNodes(extraKey(R.string.key_escape)).fetchSemanticsNodes().isNotEmpty(),
        )
        // the shell prints what it receives, line by line, control characters spelled out
        typeWord("cat")
        typeWord("-te", end = QaKeyboard.ENTER)
        screenshot("row")

        tapsSendTheirKeyOnce()
        touchesThatAreNotTapsSendNothing()
        ctrlAndAltToggleOnATap()
        anArrowSendsOneKeyOrRepeatsWhileHeld()
        aHeldArrowFollowsTheFinger()
        aBackSwipeOverTheRowSendsNothing()

        screenshot("end")
        Log.i(TAG, "terminal at the end:\n${terminalText()}")
        assertTrue("${failures.size} step(s) failed:\n${failures.joinToString("\n")}", failures.isEmpty())
    }

    private fun tapsSendTheirKeyOnce() {
        step("a tap on Esc sends Esc once") {
            assertEquals("^[", received { tap(R.string.key_escape) })
        }
        step("a tap on Tab sends Tab once") {
            assertEquals("^I", received { tap(R.string.key_tab) })
        }
        step("a tap on each symbol sends it once") {
            val keys = listOf(R.string.key_pipe, R.string.key_slash, R.string.key_tilde, R.string.key_minus)
            assertEquals("|/~-", received { keys.forEach { tap(it) } })
        }
        step("a tap on Home, then End, sends each once") {
            val sent =
                received {
                    tap(R.string.key_home)
                    tap(R.string.key_end)
                }
            assertTrue(
                "Home then End, each once, came as $sent",
                Regex("""\^\[(\[H|OH|\[1~)\^\[(\[F|OF|\[4~)""") matches sent,
            )
        }
    }

    private fun touchesThatAreNotTapsSendNothing() {
        step("a touch on Esc that slides up off the key sends nothing") {
            val key = key(R.string.key_escape)
            val sent =
                received {
                    val finger = Finger()
                    finger.down(key.center)
                    // the key shows the press while the finger rests on it
                    rest(PRESS_SHOWN_MS)
                    screenshot("esc-pressed")
                    finger.slide(key.center, Offset(key.center.x, key.top - key.height * 2))
                    finger.up()
                }
            assertEquals("", sent)
        }
        step("a touch on | that slides to ~ and lifts there sends neither") {
            assertEquals("", received { swipe(key(R.string.key_pipe).center, key(R.string.key_tilde).center) })
        }
        step("a touch on Tab that slides off and comes back before it lifts sends nothing") {
            val key = key(R.string.key_tab)
            val sent =
                received {
                    val finger = Finger()
                    finger.down(key.center)
                    finger.slide(key.center, Offset(key.center.x + key.width * 2, key.center.y))
                    finger.slide(Offset(key.center.x + key.width * 2, key.center.y), key.center)
                    finger.up()
                }
            assertEquals("", sent)
        }
        step("a swipe along the row that starts on / sends nothing") {
            assertEquals("", received { swipe(key(R.string.key_slash).center, key(R.string.key_page_up).center) })
        }
        step("a swipe along the row that starts on the left arrow sends nothing") {
            assertEquals("", received { swipe(key(R.string.key_left).center, key(R.string.key_page_down).center) })
        }
        step("a swipe along the row that starts on PgDn sends nothing") {
            assertEquals("", received { swipe(key(R.string.key_page_down).center, key(R.string.key_ctrl).center) })
        }
        step("a swipe from the row up over the terminal sends nothing") {
            val from = key(R.string.key_up).center
            assertEquals("", received { swipe(from, Offset(from.x, from.y / 2)) })
        }
    }

    private fun ctrlAndAltToggleOnATap() {
        val off = text(R.string.sticky_off)
        step("a tap on Ctrl holds it for the next key, and lets go after it") {
            val sent =
                received {
                    tap(R.string.key_ctrl)
                    assertEquals(text(R.string.sticky_once), sticky(R.string.key_ctrl))
                    typeWord("a")
                    assertEquals(off, sticky(R.string.key_ctrl))
                }
            assertEquals("^A ", sent)
        }
        step("a second tap locks Ctrl, a third lets it go") {
            val sent =
                received {
                    tap(R.string.key_ctrl)
                    tap(R.string.key_ctrl)
                    assertEquals(text(R.string.sticky_locked), sticky(R.string.key_ctrl))
                    typeWord("a")
                    typeWord("b")
                    assertEquals(text(R.string.sticky_locked), sticky(R.string.key_ctrl))
                    tap(R.string.key_ctrl)
                    assertEquals(off, sticky(R.string.key_ctrl))
                    typeWord("a")
                }
            // locked, Ctrl holds for the space bar too: Ctrl+Space is NUL
            assertEquals("^A^@^B^@a ", sent)
        }
        step("a tap on Alt holds it for the next key") {
            val sent =
                received {
                    tap(R.string.key_alt)
                    assertEquals(text(R.string.sticky_once), sticky(R.string.key_alt))
                    typeWord("a")
                    assertEquals(off, sticky(R.string.key_alt))
                }
            assertEquals("^[a ", sent)
        }
        step("a tap on Ctrl, then on Esc, sends Ctrl+Esc once and lets Ctrl go") {
            val sent =
                received {
                    tap(R.string.key_ctrl)
                    tap(R.string.key_escape)
                    assertEquals(off, sticky(R.string.key_ctrl))
                }
            assertEquals("^[[27;5;27~", sent)
        }
        step("a touch on Ctrl or Alt that slides off leaves it off") {
            val sent =
                received {
                    swipe(key(R.string.key_ctrl).center, key(R.string.key_left).center)
                    assertEquals(off, sticky(R.string.key_ctrl))
                    val alt = key(R.string.key_alt)
                    swipe(alt.center, Offset(alt.center.x, alt.top - alt.height * 2))
                    assertEquals(off, sticky(R.string.key_alt))
                    typeWord("a")
                }
            assertEquals("a ", sent)
        }
    }

    private fun anArrowSendsOneKeyOrRepeatsWhileHeld() {
        step("a quick tap on the left arrow sends one arrow") {
            assertEquals(1, arrows(received { tap(R.string.key_left) }, "D"))
        }
        step("a quick tap on each arrow and page key sends one key each") {
            val keys =
                listOf(
                    R.string.key_up,
                    R.string.key_down,
                    R.string.key_right,
                    R.string.key_page_up,
                    R.string.key_page_down,
                )
            val sent = received { keys.forEach { tap(it) } }
            val expected = Regex("""\^\[[\[O]A\^\[[\[O]B\^\[[\[O]C\^\[\[5~\^\[\[6~""")
            assertTrue("Up, Down, Right, PgUp, PgDn, each once, came as $sent", expected matches sent)
        }
        step("the left arrow held a quarter of a second sends one arrow") {
            assertEquals(1, arrows(received { hold(R.string.key_left, SHORT_HOLD_MS) }, "D"))
        }
        // had the key gone on repeating after the lift, the second the test waits there would add twenty more
        for ((millis, repeats) in listOf(MID_HOLD_MS to MID_REPEATS, LONG_HOLD_MS to LONG_REPEATS)) {
            step("the left arrow held $millis ms repeats, and stops as the finger lifts") {
                val sent =
                    received {
                        hold(R.string.key_left, millis)
                        rest(AFTER_LIFT_MS)
                    }
                val count = arrows(sent, "D")
                Log.i(TAG, "held $millis ms: $count arrows")
                assertTrue("held $millis ms, the arrow came $count times, not $repeats: $sent", count in repeats)
            }
        }
    }

    private fun aHeldArrowFollowsTheFinger() {
        step("the left arrow held, drifting inside the key, keeps repeating") {
            val key = key(R.string.key_left)
            val sent =
                received {
                    val finger = Finger()
                    finger.down(key.center)
                    rest(LONG_HOLD_MS / 2)
                    val drifted = Offset(key.center.x + key.width / 4, key.center.y + key.height / 4)
                    finger.slide(key.center, drifted)
                    rest(LONG_HOLD_MS / 2)
                    finger.up()
                }
            val count = arrows(sent, "D")
            assertTrue("held $LONG_HOLD_MS ms with a drift, the arrow came $count times: $sent", count in LONG_REPEATS)
        }
        step("the left arrow held, then slid off the key, stops repeating") {
            val key = key(R.string.key_left)
            val sent =
                received {
                    val finger = Finger()
                    finger.down(key.center)
                    rest(LONG_HOLD_MS)
                    finger.slide(key.center, Offset(key.center.x, key.top - key.height * 2))
                    // the finger stays down, off the key: a second in which a repeat still running would show
                    rest(AFTER_LIFT_MS)
                    finger.up()
                }
            val count = arrows(sent, "D")
            assertTrue("on the key $LONG_HOLD_MS ms, the arrow came $count times: $sent", count in LONG_REPEATS)
        }
    }

    private fun aBackSwipeOverTheRowSendsNothing() {
        val width = device.displayWidth.toFloat()
        val inward = width / 2
        val edges =
            listOf(
                "the left edge over Esc" to (EDGE_PX to key(R.string.key_escape).center.y),
                "the left edge over Tab" to (EDGE_PX to key(R.string.key_tab).center.y),
                "the right edge over PgUp" to (width - EDGE_PX to key(R.string.key_page_up).center.y),
                "the right edge over PgDn" to (width - EDGE_PX to key(R.string.key_page_down).center.y),
            )
        for ((where, start) in edges) {
            step("a back swipe from $where sends no key and does not go back") {
                val sent = received { swipe(Offset(start.first, start.second), Offset(inward, start.second)) }
                assertEquals("", sent)
                assertTrue(
                    "the terminal is still shown",
                    compose.onAllNodes(terminal()).fetchSemanticsNodes().isNotEmpty(),
                )
                assertTrue(
                    "the soft keyboard is still shown",
                    device.hasObject(By.desc(QaKeyboard.keyDescription("e"))),
                )
            }
        }
        step("the same swipe from the left edge over the terminal is the system's back: it hides the soft keyboard") {
            val terminal = compose.onNode(terminal()).fetchSemanticsNode().boundsInWindow
            screenshot("before-back")
            swipe(Offset(EDGE_PX, terminal.center.y), Offset(inward, terminal.center.y))
            val hidden = device.wait(Until.gone(By.desc(QaKeyboard.keyDescription("e"))), KEY_WAIT_MS)
            screenshot("after-back")
            assertTrue("the soft keyboard is gone after a back swipe over the terminal", hidden)
        }
    }

    /** Runs [body] as a step named [name]; a failure is kept for the end, with a screenshot, and the next step runs. */
    private fun step(
        name: String,
        body: () -> Unit,
    ) {
        runCatching(body)
            .onSuccess { Log.i(TAG, "step passed: $name") }
            .onFailure {
                failures += "- $name: ${it.message}"
                Log.e(TAG, "step FAILED: $name\nterminal:\n${terminalText()}", it)
                screenshot("failed-${failures.size}")
            }
    }

    private var lines = 0

    /**
     * What the shell received from [touches]: a line of its own, which starts with a marker typed on the soft keyboard
     * and ends with `z` and Enter; `cat -te` prints it back, and what stands between the marker and the `z` is what
     * the touches sent, control characters spelled out.
     */
    private fun received(touches: () -> Unit): String {
        val marker = marker(++lines)
        typeWord(marker)
        compose.waitUntil(ANSWER_WAIT_MS) { marker in screen() }
        try {
            touches()
        } finally {
            // a step that failed with Ctrl or Alt held would close its line, and type the next ones, as chords
            runCatching { letGoOfModifiers() }.onFailure { Log.e(TAG, "Ctrl and Alt could not be let go", it) }
            typeWord("z", end = QaKeyboard.ENTER)
        }
        var line: String? = null
        val came =
            runCatching {
                compose.waitUntil(ANSWER_WAIT_MS) {
                    line = printed(marker)
                    line != null
                }
            }.isSuccess
        assertTrue("the shell never printed the line $marker...z back; the terminal shows:\n${terminalText()}", came)
        Log.i(TAG, "line $marker: the shell received [$line]")
        return line.orEmpty()
    }

    /**
     * Ctrl and Alt off, wherever a step left them. Through each key's accessibility click, which acts at once: the
     * touch is what the steps test, and a step that failed on it must not keep the next ones from running.
     */
    private fun letGoOfModifiers() {
        val off = text(R.string.sticky_off)
        for (modifier in listOf(R.string.key_ctrl, R.string.key_alt)) {
            // held for one key, a click locks it and the next lets it go
            repeat(CLICKS_TO_OFF) {
                if (sticky(modifier) != off) {
                    compose.onNode(extraKey(modifier)).performSemanticsAction(SemanticsActions.OnClick)
                }
            }
        }
    }

    /** The marker of the test's line [number]: two digits, so no marker is the start of another. */
    private fun marker(number: Int) = "q%02d".format(number)

    /** The line `cat` printed back for [marker], between the marker and the closing `z$`; null until it is there. */
    private fun printed(marker: String): String? {
        val screen = screen()
        // the marker stands twice once the line is printed back: in the line as typed, then in the line `cat` printed
        val from = screen.lastIndexOf(marker).takeIf { it > screen.indexOf(marker) } ?: -1
        val end = if (from < 0) -1 else screen.indexOf("z$", from)
        // the space that ended the marker's word stands before what the touches sent
        return if (end < 0) null else screen.substring(from + marker.length, end).removePrefix(" ")
    }

    /** How many arrows of [letter] (`^[[D` or `^[OD` for left) are in [text]. */
    private fun arrows(
        text: String,
        letter: String,
    ) = Regex("""\^\[[\[O]$letter""").findAll(text).count()

    /**
     * Lets [millis] pass, for the app too: the Compose test rule owns the clock the app's timers run on (a held key's
     * among them) and moves it only as the test asks, so the test moves it on by the real time that passed, a frame
     * at a time.
     */
    private fun rest(millis: Long) {
        var last = SystemClock.uptimeMillis()
        val end = last + millis
        while (last < end) {
            SystemClock.sleep(minOf(FRAME_MS, end - last))
            val now = SystemClock.uptimeMillis()
            compose.mainClock.advanceTimeBy(now - last, ignoreFrameDuration = true)
            last = now
        }
    }

    /** One finger on the touchscreen: its events go through the system's input pipeline, as a real touch's. */
    private inner class Finger {
        private var downAt = 0L
        private var at = Offset.Zero

        fun down(where: Offset) {
            downAt = SystemClock.uptimeMillis()
            send(MotionEvent.ACTION_DOWN, where)
        }

        /** Moves from [from] to [to] in [MOVES] moves, [MOVE_MS] apart: faster than a key takes to act on a hold. */
        fun slide(
            from: Offset,
            to: Offset,
        ) {
            for (move in 1..MOVES) {
                rest(MOVE_MS)
                send(MotionEvent.ACTION_MOVE, from + (to - from) * (move.toFloat() / MOVES))
            }
        }

        /** Lifts where the finger is. */
        fun up() = send(MotionEvent.ACTION_UP, at)

        private fun send(
            action: Int,
            where: Offset,
        ) {
            at = where
            val event = MotionEvent.obtain(downAt, SystemClock.uptimeMillis(), action, where.x, where.y, 0)
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            val injected = instrumentation.uiAutomation.injectInputEvent(event, true)
            event.recycle()
            assertTrue("the system took the touch event $action at $where", injected)
        }
    }

    /** A quick tap on the row's key described [description]: down and up in its middle, [TAP_MS] apart. */
    private fun tap(description: Int) = hold(description, TAP_MS)

    /** A finger down in the middle of the key described [description], lifted there after [millis]. */
    private fun hold(
        description: Int,
        millis: Long,
    ) {
        val finger = Finger()
        finger.down(key(description).center)
        rest(millis)
        finger.up()
        compose.waitForIdle()
    }

    /** A finger down at [from], slid to [to] and lifted there. */
    private fun swipe(
        from: Offset,
        to: Offset,
    ) {
        val finger = Finger()
        finger.down(from)
        finger.slide(from, to)
        finger.up()
        compose.waitForIdle()
    }

    private fun extraKey(description: Int): SemanticsMatcher {
        val row = SemanticsMatcher.expectValue(SemanticsProperties.PaneTitle, text(R.string.extra_keys_description))
        return hasContentDescription(text(description)) and hasAnyAncestor(row)
    }

    /** Where the row's key described [description] is on the screen. */
    private fun key(description: Int): Rect = compose.onNode(extraKey(description)).fetchSemanticsNode().boundsInWindow

    /** How TalkBack says the sticky key described [description] stands. */
    private fun sticky(description: Int): String? {
        compose.waitForIdle()
        return compose
            .onNode(extraKey(description))
            .fetchSemanticsNode()
            .config
            .getOrNull(SemanticsProperties.StateDescription)
    }

    private fun signInAndOpenANewSession() {
        val host = arguments.getString(HOST).orEmpty()
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(MACHINE)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(host)
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        awaitTerminal()
        // the first attach asks for notifications (Android 13 and later): Not now, as one may answer
        val title = hasText(text(R.string.notifications_ask_title))
        val asked =
            runCatching {
                compose.waitUntil(ASK_WAIT_MS) { compose.onAllNodes(title).fetchSemanticsNodes().isNotEmpty() }
            }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
    }

    /** A tap on the terminal, once the shell's prompt is drawn on it, which brings up the test's soft keyboard. */
    private fun showKeyboard() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        compose.waitForIdle()
        val bounds = compose.onNode(terminal()).fetchSemanticsNode().boundsInWindow
        device.click(bounds.center.x.toInt(), bounds.center.y.toInt())
        softKey("e")
        // the tap opens the terminal's input method session in a coroutine, which the test rule runs only when asked
        compose.waitForIdle()
    }

    /** Types [word] on the soft keyboard, key by key, and ends it with [end]: the space bar, or Enter. */
    private fun typeWord(
        word: String,
        end: String = QaKeyboard.SPACE,
    ) {
        (word.map { it.toString() } + end).forEach { name ->
            device.waitForIdle()
            softKey(name).click()
        }
    }

    private fun softKey(name: String) =
        device.wait(Until.findObject(By.desc(QaKeyboard.keyDescription(name))), KEY_WAIT_MS).also {
            assertNotNull("the key $name on the soft keyboard", it)
        }

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    private fun awaitTerminal() =
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(terminal()).fetchSemanticsNodes().isNotEmpty() }

    /** The focused terminal's screen as TalkBack reads it: its rows, one per line. */
    private fun terminalText(): String =
        runCatching {
            compose
                .onAllNodes(terminal())
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    /** The terminal's screen as one string, a line that wraps onto the next row whole again. */
    private fun screen(): String = terminalText().lines().joinToString("")

    private fun awaitText(text: String) =
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }

    private fun text(id: Int): String = instrumentation.targetContext.getString(id)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/extra-keys-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the machine's host, ssh port, user and private key. */
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-machine"
        const val SESSION = "qa-keys"

        /** Where screenshots go on the device; scripts/test-android-e2e.sh collects them. */
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"

        /**
         * `settings get secure navigation_mode` under gesture navigation, the overlay that turns it on, and the
         * overlays of the other modes by their `navigation_mode`: three buttons, two buttons.
         */
        const val GESTURES = "2"
        const val GESTURE_OVERLAY = "com.android.internal.systemui.navbar.gestural"
        val NAVIGATION_OVERLAYS =
            mapOf(
                "0" to "com.android.internal.systemui.navbar.threebutton",
                "1" to "com.android.internal.systemui.navbar.twobutton",
            )

        /** The most clicks that take a sticky key to off: from held for one key, through locked. */
        const val CLICKS_TO_OFF = 2

        /** A quick tap's time on the key; a slide's moves and the time between them (the whole slide: 80 ms). */
        const val TAP_MS = 40L
        const val MOVES = 8
        const val MOVE_MS = 10L

        /** How far inside the screen's edge a back swipe starts. */
        const val EDGE_PX = 3f

        /** How long a pressed key is given to show its press before the screenshot. */
        const val PRESS_SHOWN_MS = 300L

        /**
         * A hold past the moment a held key acts (100 ms) and short of its first repeat (400 ms), and two holds long
         * enough to repeat, with how often the key may come by the end of each: once at 100 ms, then every 50 ms from
         * 400 ms on (8 and 18 in all), with room for a loaded emulator on both sides.
         */
        const val SHORT_HOLD_MS = 250L
        const val MID_HOLD_MS = 700L
        const val LONG_HOLD_MS = 1_200L
        val MID_REPEATS = 5..11
        val LONG_REPEATS = 14..23

        /** How long the test waits after a held key's lift, in which a repeat still running would show. */
        const val AFTER_LIFT_MS = 1_000L

        /** The step the app's clock is moved on by while the test lets time pass. */
        const val FRAME_MS = 16L

        const val KEY_WAIT_MS = 10_000L
        const val ANSWER_WAIT_MS = 15_000L
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
        const val RETRY_MS = 500L
        val ANIMATION_SCALES = listOf("window_animation_scale", "transition_animation_scale", "animator_duration_scale")
    }
}
