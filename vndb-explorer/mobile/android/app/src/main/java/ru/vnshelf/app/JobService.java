package ru.vnshelf.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/**
 * Foreground service shown while a long job (database download/import, sync) runs in the
 * embedded server. It polls http://127.0.0.1:PORT/api/status every second and mirrors the
 * progress in an ongoing notification, so the job keeps going (and stays visible) when the
 * app is in the background or the screen is off.
 */
public class JobService extends Service {
    static final String EXTRA_PORT = "port";
    private static final String CH_PROGRESS = "jobs";
    private static final String CH_DONE = "jobs_done";
    private static final int ID_PROGRESS = 1;
    private static final int ID_DONE = 2;

    static volatile boolean running = false;

    private PowerManager.WakeLock wake;
    private WifiManager.WifiLock wifi;
    private Thread worker;

    static void start(Context ctx, int port) {
        if (running || port <= 0) {
            return;
        }
        running = true;
        Intent i = new Intent(ctx, JobService.class).putExtra(EXTRA_PORT, port);
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                ctx.startForegroundService(i);
            } else {
                ctx.startService(i);
            }
        } catch (RuntimeException e) {
            // e.g. Android 12+ refuses to start a foreground service from the background:
            // the job still runs in the app, only without the notification; allow a new try later
            running = false;
        }
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        int port = intent != null ? intent.getIntExtra(EXTRA_PORT, -1) : -1;
        createChannels();
        Notification first = progressNotification("VN Shelf", "Подготовка…", "", -1);
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(ID_PROGRESS, first, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
            } else {
                startForeground(ID_PROGRESS, first);
            }
        } catch (RuntimeException e) {
            running = false;
            stopSelf();
            return START_NOT_STICKY;
        }
        acquireLocks();
        if (worker == null) {
            worker = new Thread(() -> loop(port), "vnshelf-job-watch");
            worker.start();
        }
        return START_NOT_STICKY;
    }

    private void loop(int port) {
        NotificationManager nm = getSystemService(NotificationManager.class);
        int failures = 0;
        try {
            while (failures < 30) {
                JSONObject st;
                try {
                    st = fetch("http://127.0.0.1:" + port + "/api/status");
                    failures = 0;
                } catch (Exception e) {
                    failures++;
                    Thread.sleep(1000);
                    continue;
                }
                JSONObject job = st.optJSONObject("import");
                if (job == null) {
                    break;
                }
                String title = jobTitle(job.optString("kind"));
                if (job.optBoolean("running")) {
                    JSONObject p = job.optJSONObject("progress");
                    int pct = -1;
                    String stage = "Подготовка…";
                    StringBuilder more = new StringBuilder();
                    if (p != null) {
                        pct = (int) Math.floor(p.optDouble("overall_percent", 0));
                        stage = "Этап " + p.optInt("step") + " из " + p.optInt("steps") + " · " + p.optString("label");
                        more.append(p.optString("detail"));
                        if (more.length() > 0) {
                            more.append('\n');
                        }
                        more.append("Прошло ").append(p.optString("elapsed"));
                    }
                    nm.notify(ID_PROGRESS, progressNotification(pct >= 0 ? title + " — " + pct + "%" : title, stage, more.toString(), pct));
                } else {
                    String err = job.isNull("error") ? null : job.optString("error");
                    String text;
                    if (err != null) {
                        text = "Ошибка: " + err;
                    } else if (st.optBoolean("ready")) {
                        text = "Готово: в базе " + st.optInt("vns") + " новелл";
                    } else {
                        text = "Готово";
                    }
                    nm.notify(ID_DONE, doneNotification(err != null ? title + ": не удалось" : title + ": готово", text, err != null));
                    break;
                }
                Thread.sleep(1000);
            }
        } catch (InterruptedException ignored) {
            // stopping
        } finally {
            running = false;
            releaseLocks();
            stopForeground(true);
            stopSelf();
        }
    }

    private static JSONObject fetch(String url) throws Exception {
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(3000);
        c.setReadTimeout(5000);
        try (InputStream in = c.getInputStream()) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            return new JSONObject(out.toString("UTF-8"));
        } finally {
            c.disconnect();
        }
    }

    private static String jobTitle(String kind) {
        switch (kind) {
            case "import": return "Загрузка базы VNDB";
            case "sync": return "Новые новеллы и релизы";
            case "sources": return "Индексы каталогов";
            default: return "VN Shelf";
        }
    }

    private PendingIntent openApp() {
        Intent i = new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        return PendingIntent.getActivity(this, 0, i, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    @SuppressWarnings("deprecation")
    private Notification.Builder builder(String channel) {
        return Build.VERSION.SDK_INT >= 26 ? new Notification.Builder(this, channel) : new Notification.Builder(this);
    }

    private Notification progressNotification(String title, String text, String more, int percent) {
        Notification.Builder b = builder(CH_PROGRESS)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(title)
                .setContentText(text)
                .setStyle(new Notification.BigTextStyle().bigText(more.isEmpty() ? text : text + "\n" + more))
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setShowWhen(false)
                .setContentIntent(openApp())
                .setProgress(100, Math.max(percent, 0), percent < 0);
        if (Build.VERSION.SDK_INT >= 31) {
            b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        }
        return b.build();
    }

    private Notification doneNotification(String title, String text, boolean error) {
        return builder(CH_DONE)
                .setSmallIcon(error ? android.R.drawable.stat_notify_error : android.R.drawable.stat_sys_download_done)
                .setContentTitle(title)
                .setContentText(text)
                .setStyle(new Notification.BigTextStyle().bigText(text))
                .setAutoCancel(true)
                .setContentIntent(openApp())
                .build();
    }

    private void createChannels() {
        if (Build.VERSION.SDK_INT < 26) {
            return;
        }
        NotificationManager nm = getSystemService(NotificationManager.class);
        NotificationChannel progress = new NotificationChannel(CH_PROGRESS, "Загрузка и обновление базы", NotificationManager.IMPORTANCE_LOW);
        progress.setDescription("Прогресс загрузки базы VNDB и подтягивания новых релизов");
        progress.setShowBadge(false);
        NotificationChannel done = new NotificationChannel(CH_DONE, "Завершение загрузки", NotificationManager.IMPORTANCE_DEFAULT);
        done.setDescription("Сообщение, когда база загружена или обновление не удалось");
        nm.createNotificationChannel(progress);
        nm.createNotificationChannel(done);
    }

    @SuppressWarnings("deprecation")
    private void acquireLocks() {
        try {
            PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
            wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "vnshelf:job");
            wake.acquire(3 * 60 * 60 * 1000L);
            WifiManager wm = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            wifi = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "vnshelf:job");
            wifi.acquire();
        } catch (Exception ignored) {
            // locks are best effort
        }
    }

    private void releaseLocks() {
        try {
            if (wake != null && wake.isHeld()) {
                wake.release();
            }
            if (wifi != null && wifi.isHeld()) {
                wifi.release();
            }
        } catch (Exception ignored) {
            // ignore
        }
    }

    @Override
    public void onDestroy() {
        running = false;
        if (worker != null) {
            worker.interrupt();
        }
        releaseLocks();
        super.onDestroy();
    }
}
