package com.nickalie.ngterm.android.ui.settings

import android.content.Intent
import android.net.Uri
import android.os.Looper
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.core.app.ActivityOptionsCompat
import androidx.lifecycle.viewModelScope
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.FAKE_FINGERPRINT
import com.nickalie.ngterm.android.FakeDeviceAuth
import com.nickalie.ngterm.android.FakeKeyExports
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitStore
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.fakeExport
import com.nickalie.ngterm.android.platform.AuthResult
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ExportProtection
import kotlinx.coroutines.cancel
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.IOException

/**
 * Settings' Export private key, from the key's plate to the file or the clipboard: the device's owner proves it is
 * them first, and nothing opens when they can't or won't.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp-mdpi")
class ExportKeyRouteTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private lateinit var test: TestGraph
    private val auth = FakeDeviceAuth()
    private val exports = FakeKeyExports()
    private val registry = DocumentRegistry()
    private val owner =
        object : ActivityResultRegistryOwner {
            override val activityResultRegistry = registry
        }
    private val target = Uri.parse("content://documents/ngterm-private-key")
    private val stored = "fake private key 1"

    @Before fun setUp() {
        test = TestGraph(dir.root)
        test.seedKey()
    }

    @After fun tearDown() = test.close()

    // whether Settings is the destination shown: the navigation bar's other destinations take it off the screen
    private var onSettings by mutableStateOf(true)

    private fun show(settings: SettingsViewModel? = null): ExportKeyViewModel {
        val model = ExportKeyViewModel(test.graph.keys, exports)
        compose.setContent {
            CompositionLocalProvider(LocalActivityResultRegistryOwner provides owner) {
                NgtermTheme {
                    if (onSettings) {
                        if (settings == null) {
                            SettingsRoute(test.graph, "settings-test", auth = auth, exports = model)
                        } else {
                            SettingsRoute(test.graph, "settings-test", model = settings, auth = auth, exports = model)
                        }
                    }
                }
            }
        }
        compose.awaitText("core settings-test")
        return model
    }

    private fun tap(text: String) = compose.onNodeWithText(text).performScrollTo().performClick()

    private fun field(label: String) = compose.onNode(hasText(label) and hasSetTextAction())

    private fun typePassphrase() {
        field("Passphrase").performTextReplacement("correct horse")
        field("Passphrase again").performTextReplacement("correct horse")
    }

    @Test
    fun theOwnerConfirmsThenSavesTheKeyUnderAPassphraseIntoTheFileTheyMake() {
        show()

        tap("Export private key")
        assertEquals(
            listOf("Export private key" to "Confirm it's you to export the key ngterm signs in with"),
            auth.prompts,
        )
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        tap("Save to a file")
        assertEquals(Intent.ACTION_CREATE_DOCUMENT, registry.intent.action)
        assertEquals("ngterm-private-key", registry.intent.getStringExtra(Intent.EXTRA_TITLE))
        assertTrue("nothing leaves before the file is made", exports.saved.isEmpty())
        compose.runOnUiThread { registry.dispatchResult(registry.request, target) }

        compose.awaitText("Saved. Delete the file once you've imported the key.")
        assertEquals(mapOf(target to fakeExport(stored, "correct horse")), exports.saved)
        assertTrue(exports.handed.single().all { it == 0.toByte() })
    }

    @Test
    fun theKeyIsCopiedAndBackLocksTheExportAgain() {
        show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()

        tap("Copy to the clipboard")
        compose.awaitText("Copied. Paste it", substring = true)
        assertEquals(listOf(fakeExport(stored, "correct horse")), exports.copied)
        assertEquals(fakeExport(stored, "correct horse"), exports.clipboard)

        compose.onNodeWithContentDescription("Back").performClick()
        compose.awaitText("core settings-test")
        assertEquals("the copied key left the clipboard with the screen", null, exports.clipboard)
        auth.answer = AuthResult.Cancelled
        tap("Export private key")
        compose.waitForIdle()
        compose.onNodeWithText("Save to a file").assertDoesNotExist()
        assertEquals(2, auth.prompts.size)

        auth.answer = AuthResult.Confirmed
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.awaitText("core settings-test")
        compose.onNodeWithText("Save to a file").assertDoesNotExist()
    }

    @Test
    fun aKeyMadeWhileTheExportShowsLogsNoPageTheScreenDoesNotShow() {
        val settings = SettingsViewModel(test.graph)
        try {
            show(settings)
            tap("Export private key")
            compose.awaitText(FAKE_FINGERPRINT)

            // the new key's page opens under the export, which stays on the screen
            compose.runOnIdle { settings.rotate() }
            compose.awaitStore {
                shadowOf(Looper.getMainLooper()).idle()
                !settings.keyWork.value.rotating && settings.newKey.value == null
            }
            compose.waitForIdle()
            compose.onNodeWithText("Save to a file").assertExists()
            assertEquals(
                listOf("I nav: settings page=home", "I nav: settings page=export"),
                test.logs.lines.filter { "settings page=" in it },
            )
        } finally {
            settings.viewModelScope.cancel()
        }
    }

    @Test
    fun leavingSettingsForAnotherDestinationLocksTheExportAgain() {
        val model = show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        tap("Copy to the clipboard")
        compose.awaitText("Copied. Paste it", substring = true)

        onSettings = false
        compose.waitForIdle()
        assertEquals("the unlock ended with the screen", ExportWork(), model.work.value)
        assertEquals("and the copied key left the clipboard", null, exports.clipboard)
        exports.copied.clear()
        onSettings = true

        compose.awaitText("core settings-test")
        compose.onNodeWithText("Save to a file").assertDoesNotExist()
        model.copy(ExportProtection.Passphrase("correct horse"))
        compose.waitForIdle()
        assertTrue("nothing leaves without a new confirmation", exports.copied.isEmpty())
    }

    @Test
    fun aFileScreenLeftWithoutAFileWritesNothing() {
        show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        tap("Save to a file")

        compose.runOnUiThread { registry.dispatchResult(registry.request, null) }
        compose.waitForIdle()

        assertTrue(exports.saved.isEmpty() && exports.discarded.isEmpty())
        assertTrue(test.maker.exportedFrom.isEmpty())
        compose.onNodeWithText("Save to a file").assertExists()
    }

    @Test
    fun aSecondSaveLeftWithoutAFileNoLongerSaysTheFirstOneSaved() {
        show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        tap("Save to a file")
        compose.runOnUiThread { registry.dispatchResult(registry.request, target) }
        compose.awaitText(SAVED)

        tap("Export without a passphrase")
        tap("Save to a file")
        compose.onAllNodes(hasText("Export without a passphrase"))[1].performClick()
        compose.runOnUiThread { registry.dispatchResult(registry.request, null) }
        compose.waitForIdle()

        compose.onNodeWithText(SAVED).assertDoesNotExist()
        assertEquals("only the first save wrote", mapOf(target to fakeExport(stored, "correct horse")), exports.saved)
    }

    @Test
    fun aCopyDeclinedAtItsLastQuestionNoLongerSaysTheKeyWasSaved() {
        show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        tap("Save to a file")
        compose.runOnUiThread { registry.dispatchResult(registry.request, target) }
        compose.awaitText(SAVED)

        tap("Export without a passphrase")
        tap("Copy to the clipboard")
        compose.onNodeWithText("Cancel").performClick()
        compose.waitForIdle()

        compose.onNodeWithText(SAVED).assertDoesNotExist()
        assertTrue(exports.copied.isEmpty())
    }

    @Test
    fun aSaveLeftWithoutAFileNoLongerSaysTheOneBeforeItFailed() {
        show()
        tap("Export private key")
        compose.awaitText(FAKE_FINGERPRINT)
        typePassphrase()
        exports.saveFailure = IOException("no space left")
        tap("Save to a file")
        compose.runOnUiThread { registry.dispatchResult(registry.request, target) }
        compose.awaitText("The key couldn't be exported: no space left")

        tap("Save to a file")
        compose.runOnUiThread { registry.dispatchResult(registry.request, null) }
        compose.waitForIdle()

        compose.onNodeWithText("The key couldn't be exported: no space left").assertDoesNotExist()
    }

    @Test
    fun aDeviceWithNoScreenLockIsSentToSetOneAndNeverAsked() {
        auth.secured = false
        show()

        tap("Export private key")

        compose.onNodeWithText("Set a screen lock first").assertExists()
        assertTrue(auth.prompts.isEmpty())
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("Set a screen lock first").assertDoesNotExist()
        tap("Export private key")
        compose.onNodeWithText("Open security settings").performClick()
        assertEquals(Settings.ACTION_SECURITY_SETTINGS, shadowOf(compose.activity).nextStartedActivity.action)
        compose.onNodeWithText("Save to a file").assertDoesNotExist()
        assertNull(shadowOf(compose.activity).nextStartedActivity)
    }

    @Test
    fun aDeviceThatCannotConfirmSaysWhyAndTriesAgain() {
        auth.answer = AuthResult.Failed("Too many attempts. Try again later.")
        show()

        tap("Export private key")
        compose
            .onNodeWithText(
                "The device didn't confirm the screen lock, so the key stays here: Too many attempts. Try again later.",
            ).assertExists()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("Couldn't confirm it's you").assertDoesNotExist()

        auth.answer = AuthResult.Failed("")
        tap("Export private key")
        compose.onNodeWithText("The device didn't confirm the screen lock, so the key stays here.").assertExists()
        auth.answer = AuthResult.Confirmed
        compose.onNodeWithText("Try again").performClick()

        compose.awaitText(FAKE_FINGERPRINT)
        assertEquals(3, auth.prompts.size)
    }

    private companion object {
        const val SAVED = "Saved. Delete the file once you've imported the key."
    }

    private class DocumentRegistry : ActivityResultRegistry() {
        var request = 0
        lateinit var intent: Intent

        override fun <I, O> onLaunch(
            requestCode: Int,
            contract: ActivityResultContract<I, O>,
            input: I,
            options: ActivityOptionsCompat?,
        ) {
            request = requestCode
            intent = contract.createIntent(ApplicationProvider.getApplicationContext(), input)
        }
    }
}
