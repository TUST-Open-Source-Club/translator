package com.termux.terminal;

import android.annotation.SuppressLint;
import android.os.Handler;
import android.os.Message;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * A terminal session, consisting of a process coupled to a terminal interface.
 * <p>
 * The subprocess will be executed by the constructor, and when the size is made known by a call to
 * {@link #updateSize(int, int, int, int)} terminal emulation will begin and threads will be spawned to handle the subprocess I/O.
 * All terminal emulation and callback methods will be performed on the main thread.
 * <p>
 * The child process may be exited forcefully by using the {@link #finishIfRunning()} method.
 * <p>
 * NOTE: The terminal session may outlive the EmulatorView, so be careful with callbacks!
 */
public final class TerminalSession extends TerminalOutput {

    private static final int MSG_NEW_INPUT = 1;
    private static final int MSG_PROCESS_EXITED = 4;

    public final String mHandle = UUID.randomUUID().toString();

    TerminalEmulator mEmulator;

    /**
     * A queue written to from a separate thread when the process outputs, and read by main thread to process by
     * terminal emulator.
     */
    final ByteQueue mProcessToTerminalIOQueue = new ByteQueue(4096);
    /**
     * A queue written to from the main thread due to user interaction, and read by another thread which forwards by
     * writing to the {@link #mTerminalFileDescriptor}.
     */
    final TerminalInputQueue mTerminalToProcessIOQueue = new TerminalInputQueue();
    /** Buffer to write translate code points into utf8 before writing to mTerminalToProcessIOQueue */
    private final byte[] mUtf8InputBuffer = new byte[5];

    /** Callback which gets notified when a session finishes or changes title. */
    TerminalSessionClient mClient;

    /** The pid of the shell process. 0 if not started and -1 if finished running. */
    int mShellPid;
    /** Set before killing a session so PTY workers stop independently of UI callbacks. */
    private volatile boolean mStopping;

    /** The exit status of the shell process. Only valid if ${@link #mShellPid} is -1. */
    int mShellExitStatus;

    /**
     * The file descriptor referencing the master half of a pseudo-terminal pair, resulting from calling
     * {@link JNI#createSubprocess(String, String, String[], String[], int[], int, int, int, int)}.
     */
    private int mTerminalFileDescriptor = -1;
    private final Object inputWriteLock = new Object();

    /** Set by the application for user identification of session, not by terminal. */
    public String mSessionName;

    final Handler mMainThreadHandler = new MainThreadHandler();

    private final String mShellPath;
    private final String mCwd;
    private final String[] mArgs;
    private final String[] mEnv;
    private final Integer mTranscriptRows;


    private static final String LOG_TAG = "TerminalSession";

    public TerminalSession(String shellPath, String cwd, String[] args, String[] env, Integer transcriptRows, TerminalSessionClient client) {
        this.mShellPath = shellPath;
        this.mCwd = cwd;
        this.mArgs = args;
        this.mEnv = env;
        this.mTranscriptRows = transcriptRows;
        this.mClient = client;
    }

    /**
     * @param client The {@link TerminalSessionClient} interface implementation to allow
     *               for communication between {@link TerminalSession} and its client.
     */
    public void updateTerminalSessionClient(TerminalSessionClient client) {
        mClient = client;

        if (mEmulator != null)
            mEmulator.updateTerminalSessionClient(client);
    }

    /** Inform the attached pty of the new size and reflow or initialize the emulator. */
    public void updateSize(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        if (mEmulator == null) {
            initializeEmulator(columns, rows, cellWidthPixels, cellHeightPixels);
        } else {
            if (mShellPid > 0) JNI.setPtyWindowSize(mTerminalFileDescriptor, rows, columns, cellWidthPixels, cellHeightPixels);
            mEmulator.resize(columns, rows, cellWidthPixels, cellHeightPixels);
        }
    }

    /** The terminal title as set through escape sequences or null if none set. */
    public String getTitle() {
        return (mEmulator == null) ? null : mEmulator.getTitle();
    }

    /**
     * Set the terminal emulator's window size and start terminal emulation.
     *
     * @param columns The number of columns in the terminal window.
     * @param rows    The number of rows in the terminal window.
     */
    public void initializeEmulator(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        mStopping = false;
        mEmulator = new TerminalEmulator(this, columns, rows, cellWidthPixels, cellHeightPixels, mTranscriptRows, mClient);

        int[] processId = new int[1];
        int inputFd = -1, outputFd = -1;
        try {
            mTerminalFileDescriptor = JNI.createSubprocess(mShellPath, mCwd, mArgs, mEnv, processId, rows, columns, cellWidthPixels, cellHeightPixels);
            mShellPid = processId[0];
            inputFd = JNI.duplicatePty(mTerminalFileDescriptor);
            outputFd = JNI.duplicatePty(mTerminalFileDescriptor);
        } catch (RuntimeException e) {
            if (mShellPid > 0) {
                JNI.finishSession(mShellPid);
                JNI.reapProcess(mShellPid);
            }
            if (inputFd >= 0) JNI.close(inputFd);
            if (outputFd >= 0) JNI.close(outputFd);
            if (mTerminalFileDescriptor >= 0) JNI.close(mTerminalFileDescriptor);
            mShellPid = -1;
            mStopping = true;
            mShellExitStatus = 1;
            mTerminalToProcessIOQueue.close();
            mProcessToTerminalIOQueue.close();
            mClient.logStackTraceWithMessage(LOG_TAG, "Cannot start terminal session", e);
            byte[] error = "\r\n[Cannot start process - press Enter]\r\n".getBytes(StandardCharsets.UTF_8);
            mEmulator.append(error, error.length);
            return;
        }
        // Each worker owns its descriptor. Closing the UI descriptor must never
        // let a late read/write operate on an unrelated, newly reused fd.
        final int readerFd = inputFd;
        final int writerFd = outputFd;

        Thread reader = new Thread("TermSessionInputReader[pid=" + mShellPid + "]") {
            @Override public void run() {
                byte[] buffer = new byte[4096];
                try { while (!mStopping && isRunning()) {
                    int read = JNI.readPty(readerFd, buffer);
                    if (read < 0) return;
                    if (read == 0) continue;
                    if (!mProcessToTerminalIOQueue.write(buffer, 0, read)) return;
                    mMainThreadHandler.sendEmptyMessage(MSG_NEW_INPUT);
                } } finally { JNI.close(readerFd); }
            }
        };
        reader.start();

        new Thread("TermSessionOutputWriter[pid=" + mShellPid + "]") {
            @Override public void run() {
                byte[] buffer = new byte[4096];
                try { while (!mStopping) {
                    TerminalInputQueue.Read read = mTerminalToProcessIOQueue.read(buffer);
                    if (read == null) return;
                    int count = read.count;
                    int offset = 0;
                    while (offset < count) {
                        int written;
                        synchronized (inputWriteLock) {
                            if (read.generation != mTerminalToProcessIOQueue.generation() || mStopping || !isRunning()) break;
                            written = JNI.writePty(writerFd, buffer, offset, count - offset);
                        }
                        if (written < 0) return;
                        offset += written;
                        if (written == 0) JNI.waitForPtyWrite(writerFd);
                    }
                } } finally { JNI.close(writerFd); }
            }
        }.start();

        new Thread("TermSessionWaiter[pid=" + mShellPid + "]") {
            @Override
            public void run() {
                int processExitCode = JNI.waitFor(mShellPid);
                // Kill remaining jobs so none can hold the slave PTY open. Let
                // the reader drain its final output before the completion message
                // closes the queue; a fast-exiting command must not lose its tail.
                JNI.finishSession(mShellPid);
                try { reader.join(1000); }
                    catch (InterruptedException ignored) { }
                if (reader.isAlive()) {
                    // A detached process may retain the slave despite the
                    // session scan. Never block the main thread indefinitely.
                    mStopping = true;
                    try { reader.join(250); }
                    catch (InterruptedException ignored) { }
                }
                mMainThreadHandler.sendMessage(mMainThreadHandler.obtainMessage(MSG_PROCESS_EXITED, processExitCode));
            }
        }.start();

    }

    /** Write data to the shell process. */
    @Override
    public void write(byte[] data, int offset, int count) {
        if (mShellPid <= 0 || count == 0) return;
        if (count == 1 && data[offset] == 3) {
            synchronized (inputWriteLock) {
                if (JNI.interruptTerminal(mTerminalFileDescriptor)) {
                    mTerminalToProcessIOQueue.clear();
                    return;
                }
            }
        }
        if (!mTerminalToProcessIOQueue.offer(data, offset, count)) {
            // Do not inject diagnostics into the application's terminal parser:
            // it may currently be receiving an incomplete escape sequence.
            mClient.onInputRejected(this);
        }
    }

    @Override
    public void writeText(String prefix, String text, String suffix) {
        if (mShellPid <= 0) return;
        if (prefix == null) prefix = "";
        if (text == null) text = "";
        if (suffix == null) suffix = "";
        long bytes = utf8Length(prefix) + utf8Length(text) + utf8Length(suffix);
        if (bytes > TerminalInputQueue.MAX_BYTES) {
            rejectInput();
            return;
        }
        byte[] data = (prefix + text + suffix).getBytes(StandardCharsets.UTF_8);
        write(data, 0, data.length);
    }

    @Override
    public void rejectInput() {
        if (mShellPid > 0) mClient.onInputRejected(this);
    }

    private static long utf8Length(String value) {
        long total = 0;
        for (int i = 0; i < value.length(); i++) {
            int codePoint = value.charAt(i);
            if (Character.isHighSurrogate((char) codePoint) && i + 1 < value.length()
                    && Character.isLowSurrogate(value.charAt(i + 1))) {
                codePoint = Character.toCodePoint((char) codePoint, value.charAt(++i));
            }
            // java.nio's UTF-8 encoder replaces an unpaired surrogate with '?'.
            total += codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
            if (total > TerminalInputQueue.MAX_BYTES) return total;
        }
        return total;
    }

    /** Write the Unicode code point to the terminal encoded in UTF-8. */
    public void writeCodePoint(boolean prependEscape, int codePoint) {
        if (codePoint > 1114111 || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) {
            // 1114111 (= 2**16 + 1024**2 - 1) is the highest code point, [0xD800,0xDFFF] is the surrogate range.
            throw new IllegalArgumentException("Invalid code point: " + codePoint);
        }

        int bufferPosition = 0;
        if (prependEscape) mUtf8InputBuffer[bufferPosition++] = 27;

        if (codePoint <= /* 7 bits */0b1111111) {
            mUtf8InputBuffer[bufferPosition++] = (byte) codePoint;
        } else if (codePoint <= /* 11 bits */0b11111111111) {
            /* 110xxxxx leading byte with leading 5 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11000000 | (codePoint >> 6));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else if (codePoint <= /* 16 bits */0b1111111111111111) {
            /* 1110xxxx leading byte with leading 4 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11100000 | (codePoint >> 12));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else { /* We have checked codePoint <= 1114111 above, so we have max 21 bits = 0b111111111111111111111 */
            /* 11110xxx leading byte with leading 3 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11110000 | (codePoint >> 18));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 12) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        }
        write(mUtf8InputBuffer, 0, bufferPosition);
    }

    public TerminalEmulator getEmulator() {
        return mEmulator;
    }

    /** Notify the {@link #mClient} that the screen has changed. */
    protected void notifyScreenUpdate() {
        mClient.onTextChanged(this);
    }

    /** Reset state for terminal emulator state. */
    public void reset() {
        mEmulator.reset();
        notifyScreenUpdate();
    }

    /** End the shell and every process belonging to its terminal session. */
    public void finishIfRunning() {
        // PID zero is an uninitialized session, never the caller's process group.
        synchronized (this) {
            mStopping = true;
            if (mShellPid > 0) JNI.finishSession(mShellPid);
        }
        mTerminalToProcessIOQueue.close();
    }

    /** Cleanup resources when the process exits. */
    void cleanupResources(int exitStatus) {
        synchronized (this) {
            mStopping = true;
            // waitFor leaves the leader waitable, reserving its PID until all
            // remaining jobs have been terminated. Never retain a stale PID.
            JNI.reapProcess(mShellPid);
            mShellPid = -1;
            mShellExitStatus = exitStatus;
        }

        // Stop the reader and writer threads, and close the I/O streams
        mTerminalToProcessIOQueue.close();
        mProcessToTerminalIOQueue.close();
        JNI.close(mTerminalFileDescriptor);
    }

    @Override
    public void titleChanged(String oldTitle, String newTitle) {
        mClient.onTitleChanged(this);
    }

    public synchronized boolean isRunning() {
        return mShellPid > 0;
    }

    /** Only valid if not {@link #isRunning()}. */
    public synchronized int getExitStatus() {
        return mShellExitStatus;
    }

    @Override
    public void onCopyTextToClipboard(String text) {
        mClient.onCopyTextToClipboard(this, text);
    }

    @Override
    public void onPasteTextFromClipboard() {
        mClient.onPasteTextFromClipboard(this);
    }

    @Override
    public void onBell() {
        mClient.onBell(this);
    }

    @Override
    public void onColorsChanged() {
        mClient.onColorsChanged(this);
    }

    public int getPid() {
        return mShellPid;
    }

    /** Returns the shell's working directory or null if it was unavailable. */
    public String getCwd() {
        if (mShellPid < 1) {
            return null;
        }
        try {
            final String cwdSymlink = String.format("/proc/%s/cwd/", mShellPid);
            String outputPath = new File(cwdSymlink).getCanonicalPath();
            String outputPathWithTrailingSlash = outputPath;
            if (!outputPath.endsWith("/")) {
                outputPathWithTrailingSlash += '/';
            }
            if (!cwdSymlink.equals(outputPathWithTrailingSlash)) {
                return outputPath;
            }
        } catch (IOException | SecurityException e) {
            Logger.logStackTraceWithMessage(mClient, LOG_TAG, "Error getting current directory", e);
        }
        return null;
    }

    @SuppressLint("HandlerLeak")
    class MainThreadHandler extends Handler {

        final byte[] mReceiveBuffer = new byte[4 * 1024];

        @Override
        public void handleMessage(Message msg) {
            int bytesRead = mProcessToTerminalIOQueue.read(mReceiveBuffer, false);
            if (bytesRead > 0) {
                mEmulator.append(mReceiveBuffer, bytesRead);
                notifyScreenUpdate();
            }

            if (msg.what == MSG_PROCESS_EXITED) {
                int exitCode = (Integer) msg.obj;
                cleanupResources(exitCode);

                String exitDescription = "\r\n[Process completed";
                if (exitCode > 0) {
                    // Non-zero process exit.
                    exitDescription += " (code " + exitCode + ")";
                } else if (exitCode < 0) {
                    // Negated signal.
                    exitDescription += " (signal " + (-exitCode) + ")";
                }
                exitDescription += " - press Enter]";

                byte[] bytesToWrite = exitDescription.getBytes(StandardCharsets.UTF_8);
                mEmulator.append(bytesToWrite, bytesToWrite.length);
                notifyScreenUpdate();

                mClient.onSessionFinished(TerminalSession.this);
            }
        }

    }

}
