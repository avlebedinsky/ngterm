package com.nickalie.ngterm.android.ui.components

import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentDataType
import androidx.compose.ui.semantics.contentDataType
import androidx.compose.ui.semantics.semantics

/**
 * Marks a text field whose text is a secret of ngterm's own (a key's passphrase, a private key), not the password of
 * an account: the autofill service does not fill it, does not offer to save what was typed, and suggests no strong
 * password.
 *
 * Compose gives a field with [androidx.compose.ui.text.input.KeyboardType.Password] the password content type, tells
 * the autofill service each change of its text, and commits the autofill session when the field leaves the screen: the
 * service then offers to save the passphrase. [ContentDataType.None] is Compose's way to say that a node "is not
 * autofillable": Compose then reports no change of its text and no focus on it. Put this modifier on the field's own
 * `modifier`, so it is on the same node as the text field's semantics and comes before them: the outer value wins.
 */
fun Modifier.notAutofilled(): Modifier = semantics { contentDataType = ContentDataType.None }
