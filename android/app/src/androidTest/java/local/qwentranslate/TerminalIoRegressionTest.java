package local.qwentranslate;

import android.app.Instrumentation;
import android.app.Notification;
import android.app.NotificationManager;
import android.content.Context;
import android.service.notification.StatusBarNotification;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import com.termux.terminal.TerminalSession;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import org.junit.*;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Runs real PTYs and the actual service on the test device; no model download. */
@RunWith(AndroidJUnit4.class)
public class TerminalIoRegressionTest {
    private final Instrumentation instrumentation = InstrumentationRegistry.getInstrumentation();
    private final Context context = instrumentation.getTargetContext();
    private Sessions sessions;
    private File rc;
    private byte[] saved;
    private final List<File> files = new ArrayList<>();

    private void main(Runnable action) { instrumentation.runOnMainSync(action); }
    private static byte[] read(File file) throws IOException {
        try (InputStream in = new FileInputStream(file); ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            for (int n; (n = in.read(buffer)) != -1;) out.write(buffer, 0, n);
            return out.toByteArray();
        }
    }
    private static void write(File file, byte[] bytes) throws IOException {
        try (OutputStream out = new FileOutputStream(file)) { out.write(bytes); }
    }
    private File file(String name) {
        File file = new File(sessions.home(), "test-io-" + name);
        file.delete();
        files.add(file);
        return file;
    }
    private void exists(File file) throws Exception {
        for (int i = 0; i < 200 && !file.exists(); i++) Thread.sleep(25);
        assertTrue("command did not create " + file, file.exists());
    }
    private void finished(TerminalSession session) throws Exception {
        for (int i = 0; i < 200 && session.isRunning(); i++) Thread.sleep(25);
        assertFalse("terminal process did not exit", session.isRunning());
    }
    private TerminalSession task(String command) {
        TerminalSession[] result = { null };
        main(() -> result[0] = sessions.startTranslate(Arrays.asList("-c", command)));
        return result[0];
    }
    @Before public void setup() throws Exception {
        main(() -> { sessions = Sessions.get(context); sessions.setListener(null); sessions.removeAll(); });
        rc = new File(sessions.home(), ".mkshrc");
        if (rc.exists()) saved = read(rc);
        write(rc, "export QWEN_TRANSLATE_BIN=/system/bin/sh\n".getBytes(StandardCharsets.UTF_8));
    }
    @After public void cleanup() throws Exception {
        main(() -> { sessions.setListener(null); sessions.removeAll(); });
        if (saved == null) rc.delete(); else write(rc, saved);
        for (File file : files) file.delete();
    }

    @Test public void largePasteReturnsQuicklyAndPreservesEveryByte() throws Exception {
        File ready = file("ready"), output = file("bytes");
        TerminalSession session = task("stty -echo -icanon min 1 time 0; touch '" + ready +
                "'; head -c 131072 > '" + output + "'");
        exists(ready);
        char[] chars = new char[131072];
        Arrays.fill(chars, 'x');
        String paste = new String(chars);
        long start = System.nanoTime();
        main(() -> session.getEmulator().paste(paste));
        assertTrue("paste blocked the UI", System.nanoTime() - start < 1_000_000_000L);
        finished(session);
        assertArrayEquals(paste.getBytes(StandardCharsets.UTF_8), read(output));
    }

    @Test public void controlCInterruptsEvenWithABlockedPaste() throws Exception {
        File ready = file("interrupt-ready");
        TerminalSession session = task("stty -echo; touch '" + ready + "'; sleep 60");
        exists(ready);
        char[] chars = new char[2 * 1024 * 1024];
        Arrays.fill(chars, 'x');
        main(() -> { session.getEmulator().paste(new String(chars)); session.writeCodePoint(false, 3); });
        finished(session);
        assertEquals(-2, session.getExitStatus());
    }

    @Test public void controlCRemainsLiteralInRawMode() throws Exception {
        File ready = file("raw-ready"), output = file("raw-byte");
        TerminalSession session = task("stty raw -echo; touch '" + ready + "'; head -c 1 > '" + output + "'");
        exists(ready);
        main(() -> session.writeCodePoint(false, 3));
        finished(session);
        assertArrayEquals(new byte[] { 3 }, read(output));
    }

    @Test public void immediateCloseDoesNotMissTheNewProcess() throws Exception {
        List<TerminalSession> started = new ArrayList<>();
        main(() -> {
            for (int i = 0; i < 20; i++) {
                TerminalSession session = sessions.startTranslate(Arrays.asList("-c", "sleep 60"));
                started.add(session);
                sessions.remove(session);
            }
        });
        for (TerminalSession session : started) finished(session);
    }

    @Test public void naturalShellExitAlsoTerminatesBackgroundJobs() throws Exception {
        File pidFile = file("child-pid");
        TerminalSession session = task("sleep 60 & echo $! > '" + pidFile + "'; exit 0");
        exists(pidFile);
        int child = Integer.parseInt(new String(read(pidFile), StandardCharsets.UTF_8).trim());
        finished(session);
        for (int i = 0; i < 100 && running(child); i++) Thread.sleep(25);
        assertFalse("background job outlived its terminal", running(child));
    }

    @Test public void exitingCommandDoesNotLoseTheEndOfItsOutput() throws Exception {
        TerminalSession session = task("head -c 131072 /dev/zero | tr '\\0' x; printf '\\r\\nOUTPUT_COMPLETE\\r\\n'");
        finished(session);
        String[] transcript = { null };
        main(() -> transcript[0] = session.getEmulator().getScreen().getTranscriptText());
        assertTrue("final output was discarded during exit", transcript[0].contains("OUTPUT_COMPLETE"));
    }
    private static boolean running(int pid) throws IOException {
        File stat = new File("/proc/" + pid + "/stat");
        if (!stat.exists()) return false;
        try {
            String text = new String(read(stat), StandardCharsets.UTF_8);
            char state = text.charAt(text.lastIndexOf(')') + 2);
            return state != 'Z' && state != 'X';
        } catch (FileNotFoundException gone) { return false; }
    }

    @Test public void stopAndRestartInSameMainLoopKeepsForegroundService() throws Exception {
        task("sleep 60");
        Thread.sleep(500);
        main(() -> {
            sessions.removeAll();
            sessions.startTranslate(Arrays.asList("-c", "sleep 60"));
        });
        Thread.sleep(1000);
        StatusBarNotification[] notifications = context.getSystemService(NotificationManager.class).getActiveNotifications();
        assertEquals(1, notifications.length);
        assertTrue((notifications[0].getNotification().flags & Notification.FLAG_FOREGROUND_SERVICE) != 0);
        assertEquals(1, sessions.size());
    }
}
