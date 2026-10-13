package local.qwentranslate;

import android.annotation.SuppressLint;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.system.ErrnoException;
import android.system.Os;
import android.util.Log;
import android.widget.Toast;

import com.termux.terminal.TerminalSession;
import com.termux.terminal.TerminalSessionClient;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

/**
 * The terminal sessions, kept for as long as the app process lives (the activity
 * comes and goes; {@link SessionService} keeps the process alive). Like Termux:
 * each session is a process on its own pseudo-terminal, a shell by default, and
 * a finished session stays on screen until Enter closes it.
 *
 * <p>The environment mirrors Termux's layout under the app's files directory:
 * $HOME, and $PREFIX with bin/translate and the shell's etc/mkshrc. Used on the
 * main thread only.
 */
final class Sessions implements TerminalSessionClient {
    interface Listener {
        /** Sessions were added or removed, or another one became current. */
        void onSessionsChanged();

        /** New output of {@code session} to show. */
        void onScreenUpdated(TerminalSession session);
    }

    private static final String TAG = "Sessions";
    /** Android's shell (mksh). Tests point it elsewhere. */
    static String shell = "/system/bin/sh";

    @SuppressLint("StaticFieldLeak") // holds the application context only
    private static Sessions instance;

    static Sessions get(Context context) {
        if (instance == null) instance = new Sessions(context.getApplicationContext());
        return instance;
    }

    private final Context app;
    private final File home;
    private final File prefix;
    private final List<TerminalSession> sessions = new ArrayList<>();
    /** Sessions that run one translation, not a shell. */
    private final Set<TerminalSession> tasks = new HashSet<>();
    private int current = -1;
    private Listener listener;
    // Image import workers read this after Exit is handled on the main thread.
    // Volatile makes cancellation visible without exposing the session lists to
    // background threads.
    private volatile int generation;

    int generation() { return generation; }

    private Sessions(Context app) {
        this.app = app;
        home = new File(app.getFilesDir(), "home");
        prefix = new File(app.getFilesDir(), "usr");
        install();
        SettingsMigration.migrate(app, home);
        Images.cleanUp(app);
    }

    /** $PREFIX/bin/translate and the shell setup; redone on every start, as app updates move the binary. */
    private void install() {
        File bin = new File(prefix, "bin");
        File etc = new File(prefix, "etc");
        for (File dir : new File[] {home, bin, etc}) {
            if (!dir.isDirectory() && !dir.mkdirs()) Log.w(TAG, "cannot create " + dir);
        }
        File link = new File(bin, "translate");
        //noinspection ResultOfMethodCallIgnored
        link.delete();
        try {
            Os.symlink(binary().getPath(), link.getPath());
        } catch (ErrnoException e) {
            Log.w(TAG, "cannot link " + link, e);
        }
        try (InputStream in = app.getResources().openRawResource(R.raw.mkshrc)) {
            copy(in, new File(etc, "mkshrc"));
        } catch (IOException e) {
            Log.w(TAG, "cannot write mkshrc", e);
        }
        try (OutputStream out = new FileOutputStream(new File(etc, "motd"))) {
            out.write(app.getString(R.string.motd).getBytes(StandardCharsets.UTF_8));
        } catch (IOException e) {
            Log.w(TAG, "cannot write motd", e);
        }
    }

    /** The translate executable, shipped as a native library (the only place apps may execute from). */
    File binary() {
        return new File(app.getApplicationInfo().nativeLibraryDir, "libtranslate.so");
    }

    File home() {
        return home;
    }

    void setListener(Listener l) {
        listener = l;
    }

    boolean isEmpty() {
        return sessions.isEmpty();
    }

    int size() {
        return sessions.size();
    }

    /** 0-based position of the current session. */
    int currentIndex() {
        return current;
    }

    TerminalSession current() {
        return current < 0 ? null : sessions.get(current);
    }

    List<TerminalSession> all() {
        return Collections.unmodifiableList(sessions);
    }

    /**
     * A login shell. With {@code translate}, translate's interactive mode comes first
     * (after the motd): Ctrl+D or Ctrl+C leaves it for the shell. The INT trap keeps
     * the wrapper alive when Ctrl+C ends translate (and is reset for translate itself).
     */
    TerminalSession startShell(boolean translate) {
        String[] env = environment();
        if (translate) {
            List<String> values = new ArrayList<>(java.util.Arrays.asList(env));
            values.add("QWEN_TRANSLATE_START=1");
            env = values.toArray(new String[0]);
        }
        TerminalSession s = new TerminalSession(shell, home.getPath(), new String[] {"-sh"}, env, null, this);
        s.mSessionName = "sh";
        add(s);
        return s;
    }

    /**
     * Runs translate once with these arguments (text selected or shared in another
     * app, an image) in a session of its own, which Enter closes afterwards. Earlier
     * such sessions that have finished are closed first.
     */
    TerminalSession startTranslate(List<String> args) {
        List<TerminalSession> finished = new ArrayList<>();
        for (TerminalSession t : tasks) if (!t.isRunning()) finished.add(t);
        // Positional arguments keep shared text entirely separate from shell syntax.
        String[] argv = new String[args.size() + 4];
        argv[0] = "sh";
        argv[1] = "-c";
        argv[2] = "[ ! -r \"$HOME/.mkshrc\" ] || . \"$HOME/.mkshrc\"; exec \"$QWEN_TRANSLATE_BIN\" \"$@\"";
        argv[3] = "translate";
        for (int i = 0; i < args.size(); i++) argv[i + 4] = args.get(i);
        TerminalSession s = new TerminalSession(shell, home.getPath(), argv, environment(), null, this);
        s.mSessionName = "translate";
        tasks.add(s);
        // Install the replacement before removing finished tasks: observers never see zero.
        add(s);
        for (TerminalSession t : finished) remove(t);
        return s;
    }

    private void add(TerminalSession s) {
        // An image import can finish while no Activity/view is attached.
        s.updateSize(80, 24, 8, 16);
        sessions.add(s);
        current = sessions.size() - 1;
        SessionService.update(app);
        changed();
    }

    void select(int index) {
        if (index < 0 || index >= sessions.size() || index == current) return;
        current = index;
        changed();
    }

    /** The next session, round robin. */
    void next() {
        if (sessions.size() > 1) select((current + 1) % sessions.size());
    }

    /** Ends {@code s} (if still running) and closes it; the one before it becomes current. */
    void remove(TerminalSession s) {
        int i = sessions.indexOf(s);
        if (i < 0) return;
        s.finishIfRunning();
        sessions.remove(i);
        tasks.remove(s);
        if (current >= i) current--;
        if (current < 0 && !sessions.isEmpty()) current = 0;
        SessionService.update(app);
        changed();
    }

    /** The notification's Exit button. */
    void removeAll() {
        generation++; // cancel pending image imports as well as established sessions
        for (TerminalSession s : new ArrayList<>(sessions)) remove(s);
    }

    private void changed() {
        if (listener != null) listener.onSessionsChanged();
    }

    private String[] environment() {
        Map<String, String> env = new TreeMap<>(System.getenv());
        env.put("HOME", home.getPath());
        env.put("PREFIX", prefix.getPath());
        env.put("TMPDIR", app.getCacheDir().getPath());
        String path = System.getenv("PATH");
        env.put("PATH", new File(prefix, "bin") + (path == null ? "" : ":" + path));
        // The binary itself, for when $PREFIX/bin/translate is missing (see mkshrc)
        env.put("QWEN_TRANSLATE_BIN", binary().getPath());
        // Read by every interactive mksh: prompt, history, UTF-8 line editing, motd
        env.put("ENV", new File(prefix, "etc/mkshrc").getPath());
        env.put("TERM", "xterm-256color");
        env.put("COLORTERM", "truecolor");
        env.put("LANG", "en_US.UTF-8");
        List<String> list = new ArrayList<>();
        for (Map.Entry<String, String> e : env.entrySet()) list.add(e.getKey() + "=" + e.getValue());
        return list.toArray(new String[0]);
    }

    private static void copy(InputStream in, File out) throws IOException {
        try (OutputStream os = new FileOutputStream(out)) {
            byte[] buf = new byte[8192];
            for (int n; (n = in.read(buf)) > 0; ) os.write(buf, 0, n);
        }
    }

    // TerminalSessionClient, for every session

    @Override
    public void onTextChanged(TerminalSession session) {
        if (listener != null) listener.onScreenUpdated(session);
    }

    @Override
    public void onTitleChanged(TerminalSession session) {}

    @Override
    public void onSessionFinished(TerminalSession session) {
        // Stays until Enter closes it, so its last output can still be read
        if (listener != null) listener.onScreenUpdated(session);
    }

    @Override
    public void onCopyTextToClipboard(TerminalSession session, String text) {
        app.getSystemService(ClipboardManager.class)
                .setPrimaryClip(ClipData.newPlainText(app.getString(R.string.app_name), text));
    }

    @Override
    public void onPasteTextFromClipboard(TerminalSession session) {
        ClipData clip = app.getSystemService(ClipboardManager.class).getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0 || session.getEmulator() == null) return;
        CharSequence text = clip.getItemAt(0).coerceToText(app);
        if (text != null && text.length() > 0) session.getEmulator().paste(text.toString());
    }

    @Override
    public void onInputRejected(TerminalSession session) {
        Toast.makeText(app, R.string.input_queue_full, Toast.LENGTH_LONG).show();
    }

    @Override
    public void onBell(TerminalSession session) {}

    @Override
    public void onColorsChanged(TerminalSession session) {
        if (listener != null) listener.onScreenUpdated(session);
    }

    @Override
    public void onTerminalCursorStateChange(boolean state) {}

    @Override
    public Integer getTerminalCursorStyle() {
        return null; // the emulator's default (block)
    }

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
