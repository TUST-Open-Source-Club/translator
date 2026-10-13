#include <dirent.h>
#include <errno.h>
#include <poll.h>
#include <fcntl.h>
#include <jni.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

#define TERMUX_UNUSED(x) x __attribute__((__unused__))
#ifdef __APPLE__
# define LACKS_PTSNAME_R
#endif

static void finish_session(pid_t root);

static int throw_runtime_exception(JNIEnv* env, char const* message)
{
    jclass exClass = (*env)->FindClass(env, "java/lang/RuntimeException");
    (*env)->ThrowNew(env, exClass, message);
    return -1;
}

static int create_subprocess(JNIEnv* env,
        char const* cmd,
        char const* cwd,
        char* const argv[],
        char** envp,
        int* pProcessId,
        jint rows,
        jint columns,
        jint cell_width,
        jint cell_height)
{
    int ptm = open("/dev/ptmx", O_RDWR | O_CLOEXEC | O_NONBLOCK);
    if (ptm < 0) return throw_runtime_exception(env, "Cannot open /dev/ptmx");

#ifdef LACKS_PTSNAME_R
    char* devname;
#else
    char devname[64];
#endif
    if (grantpt(ptm) || unlockpt(ptm) ||
#ifdef LACKS_PTSNAME_R
            (devname = ptsname(ptm)) == NULL
#else
            ptsname_r(ptm, devname, sizeof(devname))
#endif
       ) {
        close(ptm);
        return throw_runtime_exception(env, "Cannot grantpt()/unlockpt()/ptsname_r() on /dev/ptmx");
    }

    // Enable UTF-8 mode and disable flow control to prevent Ctrl+S from locking up the display.
    struct termios tios;
    tcgetattr(ptm, &tios);
    tios.c_iflag |= IUTF8;
    tios.c_iflag &= ~(IXON | IXOFF);
    tcsetattr(ptm, TCSANOW, &tios);

    /** Set initial winsize. */
    struct winsize sz = { .ws_row = (unsigned short) rows, .ws_col = (unsigned short) columns, .ws_xpixel = (unsigned short) (columns * cell_width), .ws_ypixel = (unsigned short) (rows * cell_height)};
    ioctl(ptm, TIOCSWINSZ, &sz);

    int initialized[2];
    if (pipe(initialized) != 0) {
        close(ptm);
        return throw_runtime_exception(env, "Cannot create subprocess initialization pipe");
    }
    pid_t pid = fork();
    if (pid < 0) {
        close(initialized[0]);
        close(initialized[1]);
        close(ptm);
        return throw_runtime_exception(env, "Fork failed");
    } else if (pid > 0) {
        close(initialized[1]);
        // A session may be closed immediately after creation. Wait until setsid
        // and the controlling terminal exist before exposing its PID to Java.
        char ready;
        while (read(initialized[0], &ready, 1) < 0 && errno == EINTR) { }
        close(initialized[0]);
        *pProcessId = (int) pid;
        return ptm;
    } else {
        // Clear signals which the Android java process may have blocked:
        sigset_t signals_to_unblock;
        sigfillset(&signals_to_unblock);
        sigprocmask(SIG_UNBLOCK, &signals_to_unblock, 0);

        close(initialized[0]);
        close(ptm);
        if (setsid() < 0) _exit(1);

        int pts = open(devname, O_RDWR);
        if (pts < 0) _exit(1);

        dup2(pts, 0);
        dup2(pts, 1);
        dup2(pts, 2);
        char ready = 1;
        while (write(initialized[1], &ready, 1) < 0 && errno == EINTR) { }
        close(initialized[1]);

        DIR* self_dir = opendir("/proc/self/fd");
        if (self_dir != NULL) {
            int self_dir_fd = dirfd(self_dir);
            struct dirent* entry;
            while ((entry = readdir(self_dir)) != NULL) {
                int fd = atoi(entry->d_name);
                if (fd > 2 && fd != self_dir_fd) close(fd);
            }
            closedir(self_dir);
        }

        clearenv();
        if (envp) for (; *envp; ++envp) putenv(*envp);

        if (chdir(cwd) != 0) {
            char* error_message;
            // No need to free asprintf()-allocated memory since doing execvp() or exit() below.
            if (asprintf(&error_message, "chdir(\"%s\")", cwd) == -1) error_message = "chdir()";
            perror(error_message);
            fflush(stderr);
        }
        execvp(cmd, argv);
        // Show terminal output about failing exec() call:
        char* error_message;
        if (asprintf(&error_message, "exec(\"%s\")", cmd) == -1) error_message = "exec()";
        perror(error_message);
        _exit(1);
    }
}

JNIEXPORT jint JNICALL Java_com_termux_terminal_JNI_createSubprocess(
        JNIEnv* env,
        jclass TERMUX_UNUSED(clazz),
        jstring cmd,
        jstring cwd,
        jobjectArray args,
        jobjectArray envVars,
        jintArray processIdArray,
        jint rows,
        jint columns,
        jint cell_width,
        jint cell_height)
{
    jsize size = args ? (*env)->GetArrayLength(env, args) : 0;
    char** argv = NULL;
    if (size > 0) {
        argv = (char**) malloc((size + 1) * sizeof(char*));
        if (!argv) return throw_runtime_exception(env, "Couldn't allocate argv array");
        for (int i = 0; i < size; ++i) {
            jstring arg_java_string = (jstring) (*env)->GetObjectArrayElement(env, args, i);
            char const* arg_utf8 = (*env)->GetStringUTFChars(env, arg_java_string, NULL);
            if (!arg_utf8) return throw_runtime_exception(env, "GetStringUTFChars() failed for argv");
            argv[i] = strdup(arg_utf8);
            (*env)->ReleaseStringUTFChars(env, arg_java_string, arg_utf8);
        }
        argv[size] = NULL;
    }

    size = envVars ? (*env)->GetArrayLength(env, envVars) : 0;
    char** envp = NULL;
    if (size > 0) {
        envp = (char**) malloc((size + 1) * sizeof(char *));
        if (!envp) return throw_runtime_exception(env, "malloc() for envp array failed");
        for (int i = 0; i < size; ++i) {
            jstring env_java_string = (jstring) (*env)->GetObjectArrayElement(env, envVars, i);
            char const* env_utf8 = (*env)->GetStringUTFChars(env, env_java_string, 0);
            if (!env_utf8) return throw_runtime_exception(env, "GetStringUTFChars() failed for env");
            envp[i] = strdup(env_utf8);
            (*env)->ReleaseStringUTFChars(env, env_java_string, env_utf8);
        }
        envp[size] = NULL;
    }

    int procId = 0;
    char const* cmd_cwd = (*env)->GetStringUTFChars(env, cwd, NULL);
    char const* cmd_utf8 = (*env)->GetStringUTFChars(env, cmd, NULL);
    int ptm = create_subprocess(env, cmd_utf8, cmd_cwd, argv, envp, &procId, rows, columns, cell_width, cell_height);
    (*env)->ReleaseStringUTFChars(env, cmd, cmd_utf8);
    (*env)->ReleaseStringUTFChars(env, cwd, cmd_cwd);

    if (argv) {
        for (char** tmp = argv; *tmp; ++tmp) free(*tmp);
        free(argv);
    }
    if (envp) {
        for (char** tmp = envp; *tmp; ++tmp) free(*tmp);
        free(envp);
    }

    int* pProcId = (int*) (*env)->GetPrimitiveArrayCritical(env, processIdArray, NULL);
    if (!pProcId) return throw_runtime_exception(env, "JNI call GetPrimitiveArrayCritical(processIdArray, &isCopy) failed");

    *pProcId = procId;
    (*env)->ReleasePrimitiveArrayCritical(env, processIdArray, pProcId, 0);

    return ptm;
}

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_setPtyWindowSize(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint fd, jint rows, jint cols, jint cell_width, jint cell_height)
{
    struct winsize sz = { .ws_row = (unsigned short) rows, .ws_col = (unsigned short) cols, .ws_xpixel = (unsigned short) (cols * cell_width), .ws_ypixel = (unsigned short) (rows * cell_height) };
    ioctl(fd, TIOCSWINSZ, &sz);
}

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_setPtyUTF8Mode(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint fd)
{
    struct termios tios;
    tcgetattr(fd, &tios);
    if ((tios.c_iflag & IUTF8) == 0) {
        tios.c_iflag |= IUTF8;
        tcsetattr(fd, TCSANOW, &tios);
    }
}

JNIEXPORT jint JNICALL Java_com_termux_terminal_JNI_waitFor(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint pid)
{
    // Keep the exited leader waitable until Java closes the session. Otherwise
    // its PID could be reused before cleanup and target an unrelated session.
    siginfo_t info;
    memset(&info, 0, sizeof(info));
    while (waitid(P_PID, (id_t) pid, &info, WEXITED | WNOWAIT) < 0) { if (errno != EINTR) return -1; }
    return info.si_code == CLD_EXITED ? info.si_status : -info.si_status;
}

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_reapProcess(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint pid)
{
    if (pid <= 0) return;
    while (waitpid(pid, NULL, 0) < 0 && errno == EINTR) { }
}

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_close(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint fileDescriptor)
{
    close(fileDescriptor);
}

JNIEXPORT jint JNICALL Java_com_termux_terminal_JNI_duplicatePty(JNIEnv* env, jclass TERMUX_UNUSED(clazz), jint fd)
{
    int copy = fcntl(fd, F_DUPFD_CLOEXEC, 0);
    return copy < 0 ? throw_runtime_exception(env, "Cannot duplicate terminal descriptor") : copy;
}

JNIEXPORT jint JNICALL Java_com_termux_terminal_JNI_readPty(JNIEnv* env, jclass TERMUX_UNUSED(clazz), jint fd, jbyteArray buffer)
{
    struct pollfd pfd = { .fd = fd, .events = POLLIN };
    int ready = poll(&pfd, 1, 200);
    if (ready == 0 || (ready < 0 && errno == EINTR)) return 0;
    if (ready < 0) return -1;
    jbyte* bytes = (*env)->GetByteArrayElements(env, buffer, NULL);
    if (!bytes) return -1;
    ssize_t count = read(fd, bytes, (size_t) (*env)->GetArrayLength(env, buffer));
    int error = errno;
    (*env)->ReleaseByteArrayElements(env, buffer, bytes, 0);
    if (count < 0 && (error == EAGAIN || error == EINTR)) return 0;
    return count <= 0 ? -1 : (jint) count;
}

JNIEXPORT jint JNICALL Java_com_termux_terminal_JNI_writePty(JNIEnv* env, jclass TERMUX_UNUSED(clazz), jint fd, jbyteArray buffer, jint offset, jint count)
{
    jbyte* bytes = (*env)->GetByteArrayElements(env, buffer, NULL);
    if (!bytes) return -1;
    ssize_t written = write(fd, bytes + offset, (size_t) count);
    int error = errno;
    (*env)->ReleaseByteArrayElements(env, buffer, bytes, JNI_ABORT);
    return written < 0 && (error == EAGAIN || error == EINTR) ? 0 : (jint) written;
}

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_waitForPtyWrite(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint fd)
{
    struct pollfd pfd = { .fd = fd, .events = POLLOUT };
    poll(&pfd, 1, 200);
}

JNIEXPORT jboolean JNICALL Java_com_termux_terminal_JNI_interruptTerminal(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint fd)
{
    struct termios attrs;
    pid_t group = tcgetpgrp(fd);
    if (group <= 0 || tcgetattr(fd, &attrs) != 0 || !(attrs.c_lflag & ISIG) || attrs.c_cc[VINTR] != 3) return JNI_FALSE;
    tcflush(fd, TCIFLUSH);
    return kill(-group, SIGINT) == 0 ? JNI_TRUE : JNI_FALSE;
}

struct session_process { pid_t pid, parent, session; int selected; };

JNIEXPORT void JNICALL Java_com_termux_terminal_JNI_finishSession(JNIEnv* TERMUX_UNUSED(env), jclass TERMUX_UNUSED(clazz), jint root)
{
    finish_session(root);
}

static void finish_session(pid_t root)
{
    if (root <= 0 || root == getpid()) return;
    // Stop the root first so it cannot spawn a new job during the snapshot.
    int root_alive = getsid(root) == root;
    if (root_alive) kill(root, SIGSTOP);
    DIR* proc = opendir("/proc");
    struct session_process* list = NULL;
    size_t count = 0;
    if (proc) {
        struct dirent* entry;
        while ((entry = readdir(proc)) != NULL) {
            char* end;
            long id = strtol(entry->d_name, &end, 10);
            if (*end || id <= 0 || id == getpid()) continue;
            char path[64], line[4096], state;
            snprintf(path, sizeof(path), "/proc/%ld/stat", id);
            FILE* file = fopen(path, "r");
            if (!file) continue;
            char* data = fgets(line, sizeof(line), file);
            fclose(file);
            char* after_name = data ? strrchr(line, ')') : NULL;
            int parent, group, session;
            if (!after_name || sscanf(after_name + 1, " %c %d %d %d", &state, &parent, &group, &session) != 4) continue;
            struct session_process* grown = realloc(list, (count + 1) * sizeof(*list));
            if (!grown) break;
            list = grown;
            list[count++] = (struct session_process) { (pid_t) id, parent, session, (id == root && root_alive) || session == root };
        }
        closedir(proc);
    }
    // Include detached descendants as well as job-control groups sharing this SID.
    int changed;
    do {
        changed = 0;
        for (size_t i = 0; i < count; i++) if (!list[i].selected) {
            for (size_t j = 0; j < count; j++) if (list[j].selected && list[i].parent == list[j].pid) {
                list[i].selected = changed = 1;
                break;
            }
        }
    } while (changed);
    for (size_t i = 0; i < count; i++) if (list[i].selected && list[i].pid != root) kill(list[i].pid, SIGKILL);
    free(list);
    if (root_alive) kill(root, SIGKILL);
}
