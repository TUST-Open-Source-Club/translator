package com.termux.terminal;

/**
 * Native methods for creating and managing pseudoterminal subprocesses. C code is in jni/termux.c.
 */
final class JNI {

    static {
        System.loadLibrary("termux");
    }

    /**
     * Create a subprocess. Differs from {@link ProcessBuilder} in that a pseudoterminal is used to communicate with the
     * subprocess.
     * <p/>
     * Callers are responsible for calling {@link #close(int)} on the returned file descriptor.
     *
     * @param cmd       The command to execute
     * @param cwd       The current working directory for the executed command
     * @param args      An array of arguments to the command
     * @param envVars   An array of strings of the form "VAR=value" to be added to the environment of the process
     * @param processId A one-element array to which the process ID of the started process will be written.
     * @return the file descriptor resulting from opening /dev/ptmx master device. The sub process will have opened the
     * slave device counterpart (/dev/pts/$N) and have it as stdint, stdout and stderr.
     */
    public static native int createSubprocess(String cmd, String cwd, String[] args, String[] envVars, int[] processId, int rows, int columns, int cellWidth, int cellHeight);

    public static native int readPty(int fd, byte[] buffer);
    public static native int duplicatePty(int fd);
    public static native int writePty(int fd, byte[] buffer, int offset, int count);
    public static native void waitForPtyWrite(int fd);

    /** Interrupt the foreground job when the PTY has ISIG enabled; otherwise send normal input. */
    public static native boolean interruptTerminal(int fd);

    /** Kill the session's process tree, including background jobs in separate process groups. */
    public static native void finishSession(int pid);

    /** Set the window size for a given pty, which allows connected programs to learn how large their screen is. */
    public static native void setPtyWindowSize(int fd, int rows, int cols, int cellWidth, int cellHeight);

    /**
     * Causes the calling thread to wait for the process associated with the receiver to finish executing.
     *
     * @return if >= 0, the exit status of the process. If < 0, the signal causing the process to stop negated.
     */
    public static native int waitFor(int processId);

    /** Reap an exited child after its terminal session has been cleaned up. */
    public static native void reapProcess(int processId);

    /** Close a file descriptor through the close(2) system call. */
    public static native void close(int fileDescriptor);

}
