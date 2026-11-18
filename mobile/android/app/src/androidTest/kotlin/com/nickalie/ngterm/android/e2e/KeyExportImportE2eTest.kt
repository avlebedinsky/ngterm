package com.nickalie.ngterm.android.e2e

import android.app.KeyguardManager
import android.content.ClipboardManager
import android.os.Build
import android.os.ParcelFileDescriptor
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import com.nickalie.ngterm.android.MainActivity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeFalse
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import java.io.File

/**
 * A key leaves the app and comes back, end to end, as when a person moves to another install of ngterm: the app makes
 * its key (the real mobile core, sealed by the device's Keystore), Settings' Export private key asks for the screen
 * lock through the system's own prompt, the key is copied under a passphrase and leaves the clipboard when the export
 * screen is closed, and Import a key reads that text back with the passphrase into the same public key; the wrong
 * passphrase is refused.
 *
 * The device gets a screen lock for the test ([PIN]) and loses it after; a device that has its own is skipped, since
 * the test can't unlock it. It needs no machine, but it needs the first run, so an app process of its own: the app's
 * stores keep what they read for the process's life, and another test that made a key there would leave this one
 * past the first run. `scripts/test-android-e2e.sh` gives each class of this package its own instrumentation run and
 * says so with its arguments ([OWN_RUN]); without them (the whole instrumented suite in one process, as CI's emulator
 * job runs it) the test is skipped.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.R)
class KeyExportImportE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()

    /** The skip, before anything is cleared or set, unless the run is this class's own. */
    private val ownRun =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no run of its own: run it through scripts/test-android-e2e.sh",
                    !InstrumentationRegistry.getArguments().getString(OWN_RUN).isNullOrBlank(),
                )
            }
        }

    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    private val screenLock =
        object : ExternalResource() {
            override fun before() {
                val keyguard = instrumentation.targetContext.getSystemService(KeyguardManager::class.java)
                assumeFalse("the device has a screen lock of its own", keyguard.isDeviceSecure)
                shell("locksettings set-pin $PIN")
                assertTrue("the test's screen lock is set", keyguard.isDeviceSecure)
            }

            override fun after() {
                shell("locksettings clear --old $PIN")
                instrumentation.targetContext.getSystemService(ClipboardManager::class.java).clearPrimaryClip()
            }
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(ownRun)
            .around(freshInstall)
            .around(screenLock)
            .around(compose)

    @Test
    fun aKeyExportedUnderAPassphraseImportsBackAsTheSameKey() {
        awaitText("Create a key")
        tap("Create a key")
        awaitText("Add this key to your machines")
        val made = publicKeyLine()
        tap("Done")
        awaitText("Settings")
        compose.onNodeWithText("Settings").performClick()

        tap("Export private key")
        unlock()
        awaitText("Save to a file")
        field("Passphrase").performTextReplacement(PASSPHRASE)
        field("Passphrase again").performTextReplacement(PASSPHRASE)
        tap("Copy to the clipboard")
        awaitText("Copied. Paste it", substring = true)
        val exported = clipboard()
        assertTrue("an OpenSSH private key was copied", exported.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----\n"))
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

        awaitText("Import a key")
        assertEquals("the copied key left the clipboard with the export screen", "", clipboard())
        tap("Import a key")
        field("Private key").performTextReplacement(exported)
        tap("Import key")
        awaitText("This key is protected: enter its passphrase.")
        field("Passphrase").performTextReplacement("not the passphrase")
        tap("Import key")
        awaitText("That passphrase doesn't open this key.")
        field("Passphrase").performTextReplacement(PASSPHRASE)
        tap("Import key")

        awaitText("Add this key to your machines")
        assertEquals("the imported key is the exported one", made, publicKeyLine())
    }

    // the system's prompt for the screen lock, answered with the test's PIN
    private fun unlock() {
        val prompt = device.wait(Until.findObject(By.text("Export private key").pkg(SYSTEM_UI)), WAIT_MS)
        assertNotNull("the system asks for the screen lock before the export opens", prompt)
        val entry = device.wait(Until.findObject(By.clazz("android.widget.EditText").pkg(SYSTEM_UI)), WAIT_MS)
        assertNotNull("the prompt takes the PIN", entry)
        shell("input text $PIN")
        shell("input keyevent KEYCODE_ENTER")
    }

    private fun publicKeyLine(): String =
        compose
            .onAllNodesWithText("ssh-ed25519 ", substring = true)
            .fetchSemanticsNodes()
            .mapNotNull { node ->
                node.config
                    .getOrNull(SemanticsProperties.Text)
                    ?.firstOrNull()
                    ?.text
            }.first { it.startsWith("ssh-ed25519 ") }

    private fun clipboard(): String {
        var text = ""
        compose.runOnUiThread {
            val manager = compose.activity.getSystemService(ClipboardManager::class.java)
            text =
                manager.primaryClip
                    ?.getItemAt(0)
                    ?.text
                    ?.toString()
                    .orEmpty()
        }
        return text
    }

    private fun awaitText(
        text: String,
        substring: Boolean = false,
    ) = compose.waitUntil(WAIT_MS) {
        compose.onAllNodesWithText(text, substring = substring).fetchSemanticsNodes().isNotEmpty()
    }

    private fun tap(text: String) {
        compose.putTheKeyboardAway(WAIT_MS)
        compose.onNodeWithText(text).performScrollTo().performClick()
    }

    private fun field(label: String) = compose.onNode(hasText(label) and hasSetTextAction())

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val WAIT_MS = 20_000L
        const val PIN = "1379"
        const val PASSPHRASE = "correct horse battery"
        const val SYSTEM_UI = "com.android.systemui"

        // an argument only scripts/test-android-e2e.sh passes, with each class's own run
        const val OWN_RUN = "e2eHost"
    }
}
