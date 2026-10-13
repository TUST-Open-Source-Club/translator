package local.qwentranslate;

import android.annotation.SuppressLint;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;

/**
 * Foreground service kept while there are sessions, like Termux's: it keeps the
 * app process, and so the processes in its terminals, alive in the background. Its
 * notification can end all sessions, or hold a wake lock so that a long download
 * or translation goes on with the screen off.
 */
public final class SessionService extends Service {
    private static final String CHANNEL = "session";
    private static final int NOTIFICATION = 1;
    private static final String ACTION_EXIT = "local.qwentranslate.EXIT";
    private static final String ACTION_WAKE_LOCK = "local.qwentranslate.WAKE_LOCK";

    /** The running instance, if any (main thread only; cleared in onDestroy). */
    @SuppressLint("StaticFieldLeak")
    private static SessionService current;

    private PowerManager.WakeLock wakeLock;

    /**
     * Follows the sessions: starts the service for the first one, updates the
     * notification, and ends the service with the last one. The service stops
     * itself: stopping it from outside before it reached startForeground() would
     * crash the app.
     */
    static void update(Context context) {
        if (current != null) {
            current.refresh();
        } else if (!Sessions.get(context).isEmpty()) {
            Intent intent = new Intent(context, SessionService.class);
            try {
                if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent);
                else context.startService(intent);
            } catch (RuntimeException e) {
                // Not allowed from the background (Android 12+): sessions work anyway
            }
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        if (current == null) {
            Notification n = notification();
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            } else {
                startForeground(NOTIFICATION, n);
            }
            current = this;
        }
        if (ACTION_EXIT.equals(action)) {
            Sessions.get(this).removeAll(); // calls update(), which stops the service
        } else if (ACTION_WAKE_LOCK.equals(action)) {
            toggleWakeLock();
        }
        refresh();
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        if (current == this) {
            current = null;
            getSystemService(NotificationManager.class).cancel(NOTIFICATION);
        }
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    /** New notification text, or the end of the service once no session is left. */
    private void refresh() {
        if (Sessions.get(this).isEmpty()) {
            // A new session may be added before onDestroy is delivered. It must
            // request a fresh start, rather than update a service already stopping.
            if (current == this) current = null;
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return;
        }
        getSystemService(NotificationManager.class).notify(NOTIFICATION, notification());
    }

    @SuppressLint("WakelockTimeout") // held on request, until the user releases it or exits
    private void toggleWakeLock() {
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager.class)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "qwentranslate:session");
            wakeLock.setReferenceCounted(false);
        }
        if (wakeLock.isHeld()) wakeLock.release();
        else wakeLock.acquire();
    }

    private Notification notification() {
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel channel = new NotificationChannel(CHANNEL,
                    getString(R.string.notification_channel), NotificationManager.IMPORTANCE_LOW);
            getSystemService(NotificationManager.class).createNotificationChannel(channel);
        }
        int count = Sessions.get(this).size();
        boolean awake = wakeLock != null && wakeLock.isHeld();
        String text = getResources().getQuantityString(R.plurals.notification_sessions, count, count);
        if (awake) text += getString(R.string.notification_wake_lock_held);
        PendingIntent open = PendingIntent.getActivity(this, 0,
                new Intent(this, TerminalActivity.class), PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder b = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, CHANNEL)
                : legacyBuilder();
        return b.setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(getString(R.string.app_name))
                .setContentText(text)
                .setContentIntent(open)
                .setOngoing(true)
                .setShowWhen(false)
                .addAction(action(ACTION_EXIT, getString(R.string.notification_exit), 1))
                .addAction(action(ACTION_WAKE_LOCK, getString(awake
                        ? R.string.notification_release_wake_lock
                        : R.string.notification_acquire_wake_lock), 2))
                .build();
    }

    private Notification.Action action(String name, String title, int requestCode) {
        PendingIntent pi = PendingIntent.getService(this, requestCode,
                new Intent(this, SessionService.class).setAction(name), PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Action.Builder(null, title, pi).build();
    }

    @SuppressWarnings("deprecation") // the only constructor before Android 8
    private Notification.Builder legacyBuilder() {
        return new Notification.Builder(this).setPriority(Notification.PRIORITY_LOW);
    }
}
