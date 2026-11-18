package com.nickalie.ngterm.android.e2e

import android.content.ClipData
import android.content.ClipboardManager
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasContentDescription
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
import com.nickalie.ngterm.android.platform.AndroidKeyExports
import com.nickalie.ngterm.android.ui.terminal.formatSize
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
import java.io.ByteArrayOutputStream
import java.io.File
import java.security.MessageDigest
import java.util.zip.GZIPOutputStream

/**
 * A clip marked sensitive is never handed to a program unasked, end to end. A program in a live session on a real
 * machine reads the phone's clipboard: with the Kitty clipboard protocol (OSC 5522) under a grant the user gave it, and
 * with OSC 52 on a machine the user set to Always allow. While the clipboard holds plain text, both reads are answered
 * with no question. While it holds a private key the app exported (the key export's own Copy, which marks the clip
 * sensitive), every read asks: the sheet says the copy is marked sensitive, shows "Hidden" over its type and size until
 * Show, has no Always button and no "don't ask again" switch, and its Allow once hands the key over for that one read.
 * The next read asks again; Deny hands over nothing. Plain text on the clipboard again is read unasked again.
 *
 * The key is put on the clipboard by [AndroidKeyExports.copy], the code behind the export screen's Copy: the export
 * screen clears its key from the clipboard when it is left, and a program in a pane that is not shown cannot read the
 * clipboard, so no path through the screens leaves the exported key on the clipboard for a shown pane's read.
 *
 * The program is a small Python reader that the test types into the session's shell (`r.py`): it puts the terminal in
 * raw mode, sends the read, waits for the answer and prints one line, `R:<label>:<status>:<bytes>:<sha-256 prefix>`,
 * which the test reads from the terminal's screen. The machine is the private `ngterm-headless` behind the private
 * `sshd` that `scripts/test-android-e2e.sh` starts; without the script's arguments the test is skipped.
 */
@RunWith(AndroidJUnit4::class)
class ClipboardSensitiveReadE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val compose = createAndroidComposeRule<MainActivity>()

    // the test's own keyboard, which passes the keys of `input text` to the app as they come; the device's keyboard
    // takes the first one as a hardware keyboard's and switches over, and the keys typed meanwhile are lost
    private val keyboards = QaKeyboards(instrumentation)

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin, no trusted machine) and an empty clipboard after the test. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }

            override fun after() {
                instrumentation.runOnMainSync { clipboard().clearPrimaryClip() }
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
            .around(keyboards)
            .around(compose)
            .around(evidence)

    private val key by lazy { String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT)) }

    @Test
    fun aSensitiveClipIsAskedForEachReadUnderAGrantAndOnAnAlwaysAllowedMachine() {
        signIn()
        addMachine()
        openANewSession()
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        keyboards.awaitShown(compose::waitForIdle)
        putTheReaderOnTheMachine()

        aKittyReadWithAGrantAsksForTheSensitiveClip()
        anAlwaysAllowedMachineAsksForTheSensitiveClip()

        screenshot("end")
        Log.i(TAG, "terminal at the end:\n${terminalText()}")
    }

    private fun aKittyReadWithAGrantAsksForTheSensitiveClip() {
        copyPlain()
        read("5522 k1 $PASSWORD")
        awaitSheet()
        awaitText(text(R.string.clipboard_preview_caption, MIME, formatSize(PLAIN.length)))
        assertEquals("the first Kitty read offers the grant switch", 1, nodes(hasText(remember())))
        screenshot("kitty-plain-asks")
        compose.onNodeWithText(remember()).performScrollTo().performClick()
        tap(text(R.string.clipboard_read_allow))
        assertEquals("the first Kitty read hands over the plain clip", done(PLAIN), result("k1"))

        read("5522 k2 $PASSWORD")
        assertEquals("the granted read gets the plain clip unasked", done(PLAIN), result("k2"))
        assertNoSheet("the granted read of a plain clip asked")

        copyKey()
        read("5522 k3 $PASSWORD")
        awaitSensitiveSheet()
        screenshot("kitty-sensitive-asks")
        tap(text(R.string.clipboard_read_allow_once))
        assertEquals("Allow once hands the key to the granted program", done(key), result("k3"))

        read("5522 k4 $PASSWORD")
        awaitSensitiveSheet()
        tap(text(R.string.clipboard_read_deny))
        assertEquals("Deny refuses the granted program", "EPERM:0:${sha(EMPTY)}", result("k4"))

        copyPlain()
        read("5522 k5 $PASSWORD")
        assertEquals("the grant still reads a plain clip unasked", done(PLAIN), result("k5"))
        assertNoSheet("the granted read of a plain clip asked after the sensitive one")
    }

    private fun anAlwaysAllowedMachineAsksForTheSensitiveClip() {
        read("52 p1")
        awaitSheet()
        awaitText(text(R.string.clipboard_preview_caption, MIME, formatSize(PLAIN.length)))
        tap(text(R.string.clipboard_read_always, MACHINE))
        assertEquals("Always allow hands over the plain clip", data(PLAIN), result("p1"))
        read("52 p2")
        assertEquals("the always-allowed machine reads a plain clip unasked", data(PLAIN), result("p2"))
        assertNoSheet("the always-allowed machine's read of a plain clip asked")

        copyKey()
        read("52 s1")
        awaitSensitiveSheet()
        screenshot("sensitive-hidden")
        assertEquals("the key shows while hidden", 0, nodes(hasText(KEY_FIRST_LINE, substring = true)))
        tap(text(R.string.clipboard_preview_show))
        awaitText(KEY_FIRST_LINE, substring = true)
        assertEquals("Hidden stays over the shown key", 0, nodes(hasText(text(R.string.clipboard_preview_hidden))))
        screenshot("sensitive-shown")
        tap(text(R.string.clipboard_preview_hide))
        awaitText(text(R.string.clipboard_preview_hidden))
        assertEquals("the key stays shown after Hide", 0, nodes(hasText(KEY_FIRST_LINE, substring = true)))
        tap(text(R.string.clipboard_read_allow_once))
        assertEquals("Allow once hands the key over", data(key), result("s1"))

        read("52 s2")
        awaitSensitiveSheet()
        tap(text(R.string.clipboard_read_deny))
        assertEquals("Deny hands over nothing", "EMPTY:0:${sha(EMPTY)}", result("s2"))

        copyPlain()
        read("52 p3")
        assertEquals("plain text is read unasked again", data(PLAIN), result("p3"))
        assertNoSheet("the always-allowed machine's read of a plain clip asked after the sensitive one")
    }

    /** The sheet for a clip marked sensitive: its words, Hidden over the type and size, Allow once and Deny alone. */
    private fun awaitSensitiveSheet() {
        awaitSheet()
        awaitText(text(R.string.clipboard_preview_hidden))
        awaitText(text(R.string.clipboard_preview_caption, MIME, formatSize(key.length)))
        val body = text(R.string.clipboard_read_sensitive_body, SESSION, MACHINE)
        assertEquals("the sheet says the copy is marked sensitive", 1, nodes(hasText(body)))
        assertEquals("the sheet offers Allow once", 1, nodes(hasText(text(R.string.clipboard_read_allow_once))))
        assertEquals("the sheet offers Deny", 1, nodes(hasText(text(R.string.clipboard_read_deny))))
        assertEquals("the sheet offers Show", 1, nodes(hasText(text(R.string.clipboard_preview_show))))
        val always = text(R.string.clipboard_read_always, MACHINE)
        assertEquals("the sheet offers Always", 0, nodes(hasText(always)))
        assertEquals("the sheet offers the grant switch", 0, nodes(hasText(remember())))
        assertEquals("the sheet offers a plain Allow", 0, nodes(hasText(text(R.string.clipboard_read_allow))))
    }

    private fun awaitSheet() = awaitText(text(R.string.clipboard_read_title, SESSION))

    private fun assertNoSheet(what: String) {
        compose.waitForIdle()
        assertEquals(what, 0, nodes(hasText(text(R.string.clipboard_read_title, SESSION))))
    }

    // the reader, typed into the shell as a gzip in base64 and unpacked into the session's working directory
    private fun putTheReaderOnTheMachine() {
        val packed = ByteArrayOutputStream()
        GZIPOutputStream(packed).use { it.write(READER.trimIndent().toByteArray()) }
        val encoded = Base64.encodeToString(packed.toByteArray(), Base64.NO_WRAP)
        // the reader's own line says it was unpacked whole; the login shell may be bash or fish: only what both read
        type("echo $encoded|base64 -d|gunzip>r.py&&python3 r.py ready")
        compose.waitUntil(CONNECT_WAIT_MS) { "R:ready:READY" in terminalText() }
    }

    private fun read(arguments: String) = type("python3 r.py $arguments")

    /** What the reader printed for [label]: its status, its byte count and its SHA-256 prefix. */
    private fun result(label: String): String {
        val line = Regex("R:$label:(\\w+:\\d+:[0-9a-f]{12})")
        var found: String? = null
        val came =
            runCatching {
                compose.waitUntil(RESULT_WAIT_MS) {
                    found = line.find(terminalText())?.groupValues?.get(1)
                    found != null
                }
            }.isSuccess
        assertTrue("the reader printed nothing for $label; the terminal shows:\n${terminalText()}", came)
        return requireNotNull(found)
    }

    private fun done(clip: String) = "DONE:${clip.toByteArray().size}:${sha(clip)}"

    private fun data(clip: String) = "DATA:${clip.toByteArray().size}:${sha(clip)}"

    private fun sha(clip: String): String =
        MessageDigest
            .getInstance("SHA-256")
            .digest(clip.toByteArray())
            .joinToString("") { "%02x".format(it) }
            .take(SHA_PREFIX)

    private fun copyPlain() =
        instrumentation.runOnMainSync { clipboard().setPrimaryClip(ClipData.newPlainText("", PLAIN)) }

    // the key export's own Copy, as the export screen does it
    private fun copyKey() =
        instrumentation.runOnMainSync {
            AndroidKeyExports(instrumentation.targetContext).copy(key.toByteArray(Charsets.US_ASCII))
        }

    private fun clipboard() = instrumentation.targetContext.getSystemService(ClipboardManager::class.java)

    private fun signIn() {
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

    // types [line] and Enter into the terminal, as a hardware keyboard does, in pieces the input command takes
    private fun type(line: String) {
        compose.waitForIdle()
        line.chunked(TYPE_CHUNK).forEach { shell("input text ${it.replace(" ", "%s")}") }
        shell("input keyevent KEYCODE_ENTER")
    }

    private fun tap(label: String) = compose.onNodeWithText(label).performScrollTo().performClick()

    private fun remember() = text(R.string.clipboard_read_remember)

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

    private fun awaitText(
        text: String,
        substring: Boolean = false,
    ) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text, substring = substring)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/clipboard-sensitive-$name.png")
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

        const val MACHINE = "qa-clip"
        const val SESSION = "qa-clip-work"

        /** What another app copied: plain text, not marked. */
        const val PLAIN = "qa plain clip"
        const val EMPTY = ""
        const val MIME = "text/plain"
        const val KEY_FIRST_LINE = "-----BEGIN OPENSSH PRIVATE KEY-----"

        /** The Kitty protocol's password the reader's grant is kept under. */
        const val PASSWORD = "qa-token"

        /**
         * The program that reads the clipboard: `r.py 52 LABEL` sends an OSC 52 read, `r.py 5522 LABEL PASSWORD` a
         * Kitty read of text/plain under PASSWORD; it prints the answer's status (OSC 52: DATA, EMPTY or NONE when
         * nothing came; Kitty: its last status), the bytes it handed over and their SHA-256 prefix.
         */
        const val READER = """
            import base64 as B,hashlib,os,re,select,sys,termios,tty
            m=sys.argv[1]
            if m=='ready':print('R:ready:READY');sys.exit()
            l=sys.argv[2]
            f=sys.stdin.fileno();o=termios.tcgetattr(f);tty.setraw(f)
            if m=='52':q=b'\x1b]52;c;?\x07'
            else:q=b'\x1b]5522;type=read:name=cWE=:pw='+B.b64encode(sys.argv[3].encode())+b';'+B.b64encode(b'text/plain')+b'\x1b\\'
            os.write(1,q);b=b''
            while select.select([f],[],[],45)[0]:
             c=os.read(f,65536)
             if not c:break
             b+=c
             if re.search(rb'(\x07|\x1b\\)$',b) and (m=='52' or re.search(rb'status=(DONE|E)',b)):break
            termios.tcsetattr(f,termios.TCSADRAIN,o)
            if m=='52':
             r=re.search(rb'\]52;\w*;([^\x07\x1b]*)',b);d=B.b64decode(r.group(1)) if r else b'';s='NONE' if not r else 'DATA' if d else 'EMPTY'
            else:
             d=b''.join(B.b64decode(x) for x in re.findall(rb'status=DATA[^;\x1b]*;([^\x07\x1b]*)',b));r=re.findall(rb'status=(\w+)',b);s=r[-1].decode() if r else 'NONE'
            print('R:%s:%s:%d:%s'%(l,s,len(d),hashlib.sha256(d).hexdigest()[:12]))
        """

        const val SHA_PREFIX = 12
        const val TYPE_CHUNK = 120
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val RESULT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
