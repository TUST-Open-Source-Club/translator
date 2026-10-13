package local.qwentranslate;

import android.content.Context;
import android.content.SharedPreferences;
import android.system.Os;
import android.util.Log;
import org.json.JSONObject;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.Iterator;
import java.util.Map;
import java.util.TreeMap;

/** Preserve the pre-Termux app's exported settings when upgrading. */
final class SettingsMigration {
    static void migrate(Context app, File home) {
        SharedPreferences prefs = app.getSharedPreferences("session", Context.MODE_PRIVATE);
        if (prefs.getBoolean("exportsMigrated", false)) return;
        File temporary = null;
        try {
            JSONObject saved = new JSONObject(prefs.getString("exports", "{}"));
            Map<String, String> values = new TreeMap<>();
            for (Iterator<String> it = saved.keys(); it.hasNext();) {
                String key = it.next();
                if (key.matches("[A-Za-z_][A-Za-z0-9_]*")) values.put(key, saved.getString(key));
            }
            if (!values.isEmpty()) {
                File rc = new File(home, ".mkshrc");
                temporary = File.createTempFile("settings-", ".tmp", home);
                try (OutputStream out = new FileOutputStream(temporary)) {
                    out.write(exports(values).getBytes(StandardCharsets.UTF_8));
                    // Existing new-style settings take precedence over migrated defaults.
                    if (rc.isFile()) try (InputStream in = new FileInputStream(rc)) {
                        byte[] buffer = new byte[8192];
                        for (int n; (n = in.read(buffer)) != -1;) out.write(buffer, 0, n);
                    }
                }
                Os.chmod(temporary.getPath(), 0600);
                Os.rename(temporary.getPath(), rc.getPath());
            }
            prefs.edit().putBoolean("exportsMigrated", true).apply();
        } catch (Exception e) {
            Log.w("SettingsMigration", "Cannot migrate saved settings; will retry next start", e);
        } finally {
            if (temporary != null) temporary.delete();
        }
    }

    static String exports(Map<String, String> values) {
        StringBuilder out = new StringBuilder("# Settings preserved from the previous app version\n");
        for (Map.Entry<String, String> e : values.entrySet()) {
            if (!e.getKey().matches("[A-Za-z_][A-Za-z0-9_]*") || e.getValue().indexOf('\0') >= 0) continue;
            out.append("export ").append(e.getKey()).append("='")
                    .append(e.getValue().replace("'", "'\\''")).append("'\n");
        }
        return out.append('\n').toString();
    }
}
