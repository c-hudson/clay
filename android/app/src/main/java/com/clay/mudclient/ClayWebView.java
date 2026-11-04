package com.clay.mudclient;

import android.content.Context;
import android.text.InputType;
import android.util.AttributeSet;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.webkit.WebView;

/**
 * The app's WebView, with one addition: while a live shell world is current, the soft
 * keyboard is told this is a field it must not compose in (a visible-password field with
 * no suggestions, as Termux does), so it hands each letter over as it is typed and the
 * page sends it straight to the shell. Otherwise a keyboard like Gboard builds a whole
 * word in the input box and commits it only at a space. app.js turns it on and off
 * (AndroidInterface.setShellKeyboard, from setShellInputMode).
 */
public class ClayWebView extends WebView {
    private boolean charInput = false;

    public ClayWebView(Context context) {
        super(context);
    }

    public ClayWebView(Context context, AttributeSet attrs) {
        super(context, attrs);
    }

    public ClayWebView(Context context, AttributeSet attrs, int defStyleAttr) {
        super(context, attrs, defStyleAttr);
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        InputConnection ic = super.onCreateInputConnection(outAttrs);
        if (charInput && ic != null) {
            outAttrs.inputType = InputType.TYPE_CLASS_TEXT
                | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
                | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            outAttrs.imeOptions |= EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING;
        }
        return ic;
    }

    /** Letter-at-a-time typing on or off; the keyboard picks it up at once. UI thread. */
    public void setCharInput(boolean on) {
        if (charInput == on) return;
        charInput = on;
        InputMethodManager imm =
            (InputMethodManager) getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) imm.restartInput(this);
    }
}
