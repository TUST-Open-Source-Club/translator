package local.qwentranslate;

import android.Manifest;
import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.content.res.TypedArray;
import android.graphics.Insets;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

import com.termux.terminal.TerminalSession;
import com.termux.view.TerminalView;
import com.termux.view.TerminalViewClient;

import java.io.File;
import java.io.IOException;
import java.util.Arrays;

/**
 * The terminal: Termux's {@link TerminalView} showing the current session, and a
 * row of extra keys below it, as in Termux.
 */
public final class TerminalActivity extends Activity implements Sessions.Listener, TerminalViewClient {
    private static final int REQUEST_IMAGE = 1;
    private static final int REQUEST_NOTIFICATIONS = 2;

    private Sessions sessions;
    private SharedPreferences prefs;
    private TerminalView terminal;
    private TextView sessionKey;
    private int fontSize;
    private int minFontSize;
    private int maxFontSize;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        setContentView(R.layout.terminal);
        sessions = Sessions.get(this);
        prefs = getSharedPreferences("ui", MODE_PRIVATE);
        terminal = findViewById(R.id.terminal);
        fitInsets(findViewById(R.id.root));

        terminal.setTerminalViewClient(this);
        // Font sizes in pixels, as in Termux: 12dp by default (the zoom step is 2).
        // setTextSize() also creates the renderer (monospace), which much of the view needs.
        int dip = Math.max(1, Math.round(TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, 1, getResources().getDisplayMetrics())));
        minFontSize = 4 * dip;
        maxFontSize = 64 * dip;
        fontSize = Math.max(minFontSize, Math.min(maxFontSize, prefs.getInt("fontSize", 12 * dip)));
        terminal.setTextSize(fontSize);

        LinearLayout keys = findViewById(R.id.keys);
        addKey(keys, getString(R.string.key_esc), () -> key(KeyEvent.KEYCODE_ESCAPE));
        addKey(keys, getString(R.string.key_interrupt), () -> codePoint(3));
        addKey(keys, getString(R.string.key_eof), () -> codePoint(4));
        addKey(keys, getString(R.string.key_tab), () -> key(KeyEvent.KEYCODE_TAB));
        addKey(keys, getString(R.string.key_up), () -> key(KeyEvent.KEYCODE_DPAD_UP));
        addKey(keys, getString(R.string.key_down), () -> key(KeyEvent.KEYCODE_DPAD_DOWN));
        addKey(keys, getString(R.string.key_left), () -> key(KeyEvent.KEYCODE_DPAD_LEFT));
        addKey(keys, getString(R.string.key_right), () -> key(KeyEvent.KEYCODE_DPAD_RIGHT));
        addKey(keys, getString(R.string.key_image), this::pickImage);
        addKey(keys, getString(R.string.key_paste), () -> {
            TerminalSession s = terminal.getCurrentSession();
            if (s != null && s.isRunning()) sessions.onPasteTextFromClipboard(s);
        });
        // Tap: next session; long press: a new shell
        sessionKey = addKey(keys, "", () -> sessions.next());
        sessionKey.setOnLongClickListener(v -> {
            sessions.startShell(false);
            return true;
        });

        if (sessions.isEmpty()) {
            String action = getIntent().getAction();
            boolean external = Intent.ACTION_SEND.equals(action) || Intent.ACTION_PROCESS_TEXT.equals(action);
            sessions.startShell(!external);
        }
        // Not again when recreated: the request was handled already
        if (state == null) handleIntent(getIntent());
        askForNotifications();
        terminal.requestFocus();
    }

    @Override
    protected void onStart() {
        super.onStart();
        sessions.setListener(this);
        onSessionsChanged();
    }

    @Override
    protected void onStop() {
        sessions.setListener(null);
        super.onStop();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    // Sessions.Listener

    @Override
    public void onSessionsChanged() {
        if (sessions.isEmpty()) {
            // The last session was closed (exit, then Enter)
            finish();
            return;
        }
        TerminalSession s = sessions.current();
        if (terminal.getCurrentSession() != s) terminal.attachSession(s);
        terminal.onScreenUpdated();
        sessionKey.setText(getString(R.string.key_session, sessions.currentIndex() + 1, sessions.size()));
    }

    @Override
    public void onScreenUpdated(TerminalSession session) {
        if (session == terminal.getCurrentSession()) terminal.onScreenUpdated();
    }

    // Requests from other apps

    /** Text selected in another app ("Qwen Translate" in its menu), or shared text / image. */
    private void handleIntent(Intent intent) {
        String action = intent.getAction();
        if (Intent.ACTION_PROCESS_TEXT.equals(action)) {
            translate(intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT));
        } else if (Intent.ACTION_SEND.equals(action)) {
            String type = intent.getType();
            Uri image = type != null && type.startsWith("image/") ? sharedStream(intent) : null;
            if (image != null) translateImage(image);
            else translate(intent.getCharSequenceExtra(Intent.EXTRA_TEXT));
        }
    }

    private void translate(CharSequence text) {
        String t = text == null ? "" : text.toString().trim();
        // "--": text starting with a dash is still text
        if (!t.isEmpty()) sessions.startTranslate(Arrays.asList("--", t));
    }

    /** Copies the image off the main thread, then translates it in a session of its own. */
    private void translateImage(Uri uri) {
        Context app = getApplicationContext();
        Handler main = new Handler(Looper.getMainLooper());
        int generation = sessions.generation();
        if (!Images.execute(() -> {
            try {
                // Do not start another potentially large decode after Exit has
                // cancelled the batch while this job was waiting in the queue.
                if (Sessions.get(app).generation() != generation) return;
                File image = Images.importImage(app, uri);
                main.post(() -> {
                    Sessions owner = Sessions.get(app);
                    if (owner.generation() == generation) owner.startTranslate(Arrays.asList("--image", image.getPath()));
                    else image.delete(); // Exit cancels imports that were still in flight.
                });
            } catch (IOException | RuntimeException | OutOfMemoryError e) {
                String msg = app.getString(R.string.image_failed, String.valueOf(e.getMessage()));
                main.post(() -> Toast.makeText(app, msg, Toast.LENGTH_LONG).show());
            }
        })) Toast.makeText(app, app.getString(R.string.image_failed, "too many pending images"), Toast.LENGTH_LONG).show();
    }

    @SuppressWarnings("deprecation") // the typed variant needs Android 13
    private static Uri sharedStream(Intent intent) {
        return Build.VERSION.SDK_INT >= 33
                ? intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri.class)
                : intent.getParcelableExtra(Intent.EXTRA_STREAM);
    }

    private void pickImage() {
        // The photo picker needs no storage permission; older versions use a document picker
        Intent pick = Build.VERSION.SDK_INT >= 33
                ? new Intent(MediaStore.ACTION_PICK_IMAGES)
                : new Intent(Intent.ACTION_GET_CONTENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);
        try {
            startActivityForResult(pick, REQUEST_IMAGE);
        } catch (ActivityNotFoundException e) {
            Toast.makeText(this, R.string.no_image_picker, Toast.LENGTH_SHORT).show();
        }
    }

    @Override
    @SuppressWarnings("deprecation") // the framework Activity has no result API besides this one
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == REQUEST_IMAGE && result == RESULT_OK && data != null && data.getData() != null) {
            translateImage(data.getData());
        }
    }

    /** Android 13+: once, so the sessions' notification can be shown. */
    private void askForNotifications() {
        if (Build.VERSION.SDK_INT < 33 || prefs.getBoolean("askedNotifications", false)) return;
        prefs.edit().putBoolean("askedNotifications", true).apply();
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, REQUEST_NOTIFICATIONS);
        }
    }

    // Extra keys

    private TextView addKey(LinearLayout row, String label, Runnable action) {
        TextView key = new TextView(this);
        key.setText(label);
        key.setTextColor(getColor(R.color.term_fg));
        key.setTypeface(Typeface.MONOSPACE);
        key.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14);
        key.setGravity(Gravity.CENTER);
        key.setMinWidth(dp(44));
        key.setMinHeight(dp(44));
        key.setPadding(dp(10), 0, dp(10), 0);
        TypedArray a = obtainStyledAttributes(new int[] {android.R.attr.selectableItemBackground});
        key.setBackground(a.getDrawable(0));
        a.recycle();
        // Keys must not take the focus (and the keyboard) away from the terminal
        key.setFocusable(false);
        key.setOnClickListener(v -> action.run());
        row.addView(key);
        return key;
    }

    /** A key as the terminal sends it (honours the cursor key mode programs set). */
    private void key(int keyCode) {
        TerminalSession s = terminal.getCurrentSession();
        if (s != null && s.isRunning() && s.getEmulator() != null) terminal.handleKeyCode(keyCode, 0);
    }

    /** A control character: 3 is ^C (interrupt), 4 is ^D (end of input). */
    private void codePoint(int c) {
        TerminalSession s = terminal.getCurrentSession();
        if (s != null && s.isRunning()) s.writeCodePoint(false, c);
    }

    private void changeFontSize(boolean increase) {
        fontSize = Math.max(minFontSize, Math.min(maxFontSize, fontSize + (increase ? 2 : -2)));
        prefs.edit().putInt("fontSize", fontSize).apply();
        terminal.setTextSize(fontSize);
    }

    private void showKeyboard() {
        terminal.requestFocus();
        getSystemService(InputMethodManager.class).showSoftInput(terminal, 0);
    }

    private int dp(int v) {
        return Math.round(v * getResources().getDisplayMetrics().density);
    }

    /** Android 11+: draw edge to edge, keeping the content clear of system bars and keyboard. */
    @SuppressWarnings("deprecation") // only called where edge to edge is not yet the default
    private void fitInsets(View root) {
        if (Build.VERSION.SDK_INT < 30) return; // older: adjustResize between the bars
        if (Build.VERSION.SDK_INT < 35) getWindow().setDecorFitsSystemWindows(false);
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            Insets i = insets.getInsets(WindowInsets.Type.systemBars()
                    | WindowInsets.Type.ime() | WindowInsets.Type.displayCutout());
            v.setPadding(i.left, i.top, i.right, i.bottom);
            return WindowInsets.CONSUMED;
        });
    }

    // TerminalViewClient

    @Override
    public float onScale(float scale) {
        // As in Termux: past a threshold, step the font size and restart the gesture
        if (scale < 0.9f || scale > 1.1f) {
            changeFontSize(scale > 1f);
            return 1.0f;
        }
        return scale;
    }

    @Override
    public void onSingleTapUp(MotionEvent e) {
        TerminalSession s = terminal.getCurrentSession();
        if (s == null || s.getEmulator() == null || !s.getEmulator().isMouseTrackingActive()) showKeyboard();
    }

    @Override
    public boolean shouldBackButtonBeMappedToEscape() {
        return false;
    }

    @Override
    public boolean shouldEnforceCharBasedInput() {
        return false;
    }

    @Override
    public boolean shouldUseCtrlSpaceWorkaround() {
        return false;
    }

    @Override
    public boolean isTerminalViewSelected() {
        return true;
    }

    @Override
    public void copyModeChanged(boolean copyMode) {}

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent e, TerminalSession session) {
        // Enter closes a finished session ("[Process completed - press Enter]")
        if (keyCode == KeyEvent.KEYCODE_ENTER && session != null && !session.isRunning()) {
            sessions.remove(session);
            return true;
        }
        return false;
    }

    @Override
    public boolean onKeyUp(int keyCode, KeyEvent e) {
        return false;
    }

    @Override
    public boolean onLongPress(MotionEvent event) {
        return false;
    }

    @Override
    public boolean readControlKey() {
        return false;
    }

    @Override
    public boolean readAltKey() {
        return false;
    }

    @Override
    public boolean readShiftKey() {
        return false;
    }

    @Override
    public boolean readFnKey() {
        return false;
    }

    @Override
    public boolean onCodePoint(int codePoint, boolean ctrlDown, TerminalSession session) {
        // Some keyboards send Enter as text
        if ((codePoint == '\r' || codePoint == '\n') && session != null && !session.isRunning()) {
            sessions.remove(session);
            return true;
        }
        return false;
    }

    @Override
    public void onEmulatorSet() {}

    @Override
    public void logError(String tag, String message) {
        Log.e(tag, message);
    }

    @Override
    public void logWarn(String tag, String message) {
        Log.w(tag, message);
    }

    @Override
    public void logInfo(String tag, String message) {
        Log.i(tag, message);
    }

    @Override
    public void logDebug(String tag, String message) {
        Log.d(tag, message);
    }

    @Override
    public void logVerbose(String tag, String message) {
        Log.v(tag, message);
    }

    @Override
    public void logStackTraceWithMessage(String tag, String message, Exception e) {
        Log.e(tag, message, e);
    }

    @Override
    public void logStackTrace(String tag, Exception e) {
        Log.e(tag, "", e);
    }
}
