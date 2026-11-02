package ru.vnshelf.app;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.WindowManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.TextView;

/**
 * Thin shell: the Rust library (libvnshelf.so) runs the same local server as the desktop
 * version on 127.0.0.1 and this activity shows it in a WebView. External links open in the
 * browser. All data stays in the app's private storage.
 */
public class MainActivity extends Activity {
    static {
        System.loadLibrary("vnshelf");
    }

    /** Starts the embedded server (once per process) and returns its port, or -1. */
    private static native int startServer(String filesDir);

    private WebView web;
    private int port = -1;

    @SuppressLint("SetJavaScriptEnabled")
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        TextView splash = new TextView(this);
        splash.setText(R.string.starting);
        splash.setTextColor(Color.parseColor("#9196AD"));
        splash.setBackgroundColor(Color.parseColor("#12131C"));
        splash.setGravity(Gravity.CENTER);
        splash.setTextSize(18);
        setContentView(splash);

        web = new WebView(this);
        web.setBackgroundColor(Color.parseColor("#12131C"));
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setDatabaseEnabled(true);
        s.setMediaPlaybackRequiresUserGesture(true);
        s.setUserAgentString(s.getUserAgentString() + " VNShelfApp/" + BuildConfigVersion.name(this));
        // Lets the page keep the screen on during a database download/import.
        web.addJavascriptInterface(new NativeBridge(), "VNShelfNative");
        web.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri u = request.getUrl();
                if ("127.0.0.1".equals(u.getHost())) {
                    return false;
                }
                openExternal(u);
                return true;
            }
        });

        final String dir = getFilesDir().getAbsolutePath();
        final Handler main = new Handler(Looper.getMainLooper());
        // Loading the catalog takes a moment: never block the UI thread.
        new Thread(() -> {
            int p = startServer(dir);
            main.post(() -> {
                port = p;
                if (p <= 0) {
                    splash.setText(R.string.start_failed);
                    return;
                }
                setContentView(web);
                if (state != null && web.restoreState(state) != null) {
                    return;
                }
                web.loadUrl("http://127.0.0.1:" + p + "/");
            });
        }, "vnshelf-start").start();
    }

    /** Methods callable from the page as window.VNShelfNative.* (only our own 127.0.0.1 page is ever loaded). */
    final class NativeBridge {
        /** A long job runs in the server: show it in a notification (foreground service). */
        @JavascriptInterface
        public void watchJob() {
            if (JobService.running) {
                return;
            }
            runOnUiThread(() -> {
                if (android.os.Build.VERSION.SDK_INT >= 33
                        && checkSelfPermission("android.permission.POST_NOTIFICATIONS") != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    requestPermissions(new String[]{"android.permission.POST_NOTIFICATIONS"}, 1);
                }
                JobService.start(MainActivity.this, port);
            });
        }

        /** In-app update: "ok", "busy" or "need_permission". */
        @JavascriptInterface
        public String startUpdate(String url) {
            return Updater.start(MainActivity.this, url);
        }

        @JavascriptInterface
        public String updateStatus() {
            return Updater.status();
        }

        @JavascriptInterface
        public void keepAwake(boolean on) {
            runOnUiThread(() -> {
                if (on) {
                    getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                } else {
                    getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                }
            });
        }
    }

    private void openExternal(Uri u) {
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, u));
        } catch (ActivityNotFoundException ignored) {
            // no browser installed
        }
    }

    @Override
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        if (web != null && port > 0) {
            web.saveState(out);
        }
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (web != null && web.canGoBack()) {
            web.goBack();
        } else {
            super.onBackPressed();
        }
    }

    /** Version name without depending on the generated BuildConfig class. */
    static final class BuildConfigVersion {
        static String name(Activity a) {
            try {
                return a.getPackageManager().getPackageInfo(a.getPackageName(), 0).versionName;
            } catch (Exception e) {
                return "?";
            }
        }
    }
}
