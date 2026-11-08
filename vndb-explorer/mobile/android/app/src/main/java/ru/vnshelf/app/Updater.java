package ru.vnshelf.app;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;

import org.json.JSONObject;

import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/**
 * In-app update: downloads the new APK straight into a PackageInstaller session and lets
 * Android show its "Install" confirmation, so the user never leaves the app.
 * The same signing key is required (see mobile/android/app/build.gradle).
 */
public final class Updater {
    private static volatile String state = "idle"; // idle | downloading | installing | done | error
    private static volatile long done = 0;
    private static volatile long total = 0;
    private static volatile String error = null;

    static String status() {
        try {
            JSONObject o = new JSONObject();
            o.put("state", state);
            o.put("done", done);
            o.put("total", total);
            if (error != null) {
                o.put("error", error);
            }
            return o.toString();
        } catch (Exception e) {
            return "{\"state\":\"error\"}";
        }
    }

    /** @return "ok", "busy" or "need_permission" (the "install unknown apps" screen was opened). */
    static String start(Activity activity, String url) {
        if ("downloading".equals(state) || "installing".equals(state)) {
            return "busy";
        }
        // only the public release asset on GitHub (https, redirects to the download host)
        if (url == null || !url.startsWith("https://github.com/")) {
            state = "error";
            error = "bad url";
            return "ok";
        }
        if (Build.VERSION.SDK_INT >= 26 && !activity.getPackageManager().canRequestPackageInstalls()) {
            activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + activity.getPackageName())));
            return "need_permission";
        }
        state = "downloading";
        done = 0;
        total = 0;
        error = null;
        Context app = activity.getApplicationContext();
        new Thread(() -> run(app, url), "vnshelf-update").start();
        return "ok";
    }

    private static void run(Context ctx, String url) {
        PackageInstaller.Session session = null;
        try {
            HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
            c.setInstanceFollowRedirects(true);
            c.setConnectTimeout(15000);
            c.setReadTimeout(120000);
            c.setRequestProperty("Accept", "application/octet-stream");
            int code = c.getResponseCode();
            if (code != 200) {
                throw new IllegalStateException("HTTP " + code + (code == 404 ? " (файл не найден — откройте страницу релизов)" : ""));
            }
            total = c.getContentLengthLong();
            PackageInstaller pi = ctx.getPackageManager().getPackageInstaller();
            PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
            params.setAppPackageName(ctx.getPackageName());
            if (total > 0) {
                params.setSize(total);
            }
            int id = pi.createSession(params);
            session = pi.openSession(id);
            try (InputStream in = c.getInputStream(); OutputStream out = session.openWrite("update.apk", 0, total > 0 ? total : -1)) {
                byte[] buf = new byte[64 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) {
                    out.write(buf, 0, n);
                    done += n;
                }
                session.fsync(out);
            }
            int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
            PendingIntent pending = PendingIntent.getBroadcast(ctx, id,
                    new Intent(ctx, Receiver.class).setPackage(ctx.getPackageName()), flags);
            state = "installing";
            session.commit(pending.getIntentSender());
            session.close();
            session = null;
        } catch (Exception e) {
            if (session != null) {
                session.abandon();
            }
            error = e.getMessage();
            state = "error";
        }
    }

    /** Receives the install result; asks the user to confirm when Android needs it. */
    public static final class Receiver extends BroadcastReceiver {
        @Override
        public void onReceive(Context context, Intent intent) {
            int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
            if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                if (confirm != null) {
                    confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                    context.startActivity(confirm);
                }
            } else if (status == PackageInstaller.STATUS_SUCCESS) {
                state = "done";
            } else {
                error = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                if (error == null) {
                    error = "установка отменена";
                }
                state = "error";
            }
        }
    }
}
