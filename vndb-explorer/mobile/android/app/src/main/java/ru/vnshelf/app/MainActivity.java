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

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

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

    private static final int REQ_SAVE_FILE = 41;
    private static final int REQ_OPEN_FILE = 42;
    /** Largest file the page may open (the shelves file is a few hundred KB at most). */
    private static final int MAX_OPEN_BYTES = 8 * 1024 * 1024;
    /** Text waiting for the user to pick where to save it. */
    private String pendingSave;

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

            /** The WebView renderer crashed or was killed for memory: rebuild instead of crashing the app. */
            @Override
            public boolean onRenderProcessGone(WebView view, android.webkit.RenderProcessGoneDetail detail) {
                if (view == web) {
                    web = null;
                    recreate();
                }
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
                String home = "http://127.0.0.1:" + p + "/";
                // restore the page only if it belongs to this server (the port may differ after a restart)
                if (state != null && web.restoreState(state) != null && web.getUrl() != null && web.getUrl().startsWith(home)) {
                    return;
                }
                web.loadUrl(home);
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

        /** "Save as" dialog; the page gets window.VNShelfFileSaved(true|false). */
        @JavascriptInterface
        public void saveFile(String name, String text) {
            runOnUiThread(() -> {
                pendingSave = text;
                Intent i = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                i.addCategory(Intent.CATEGORY_OPENABLE);
                i.setType("application/json");
                i.putExtra(Intent.EXTRA_TITLE, name);
                try {
                    startActivityForResult(i, REQ_SAVE_FILE);
                } catch (ActivityNotFoundException e) {
                    pendingSave = null;
                    callPage("VNShelfFileSaved", "false");
                }
            });
        }

        /** "Open file" dialog; the page gets window.VNShelfFileOpened(text or null). */
        @JavascriptInterface
        public void openFile() {
            runOnUiThread(() -> {
                Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                i.addCategory(Intent.CATEGORY_OPENABLE);
                i.setType("*/*");
                try {
                    startActivityForResult(i, REQ_OPEN_FILE);
                } catch (ActivityNotFoundException e) {
                    callPage("VNShelfFileOpened", "null");
                }
            });
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

    @Override
    @SuppressWarnings("deprecation")
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        Uri uri = result == RESULT_OK && data != null ? data.getData() : null;
        if (request == REQ_SAVE_FILE) {
            boolean ok = false;
            if (uri != null && pendingSave != null) {
                try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                    if (out != null) {
                        out.write(pendingSave.getBytes(StandardCharsets.UTF_8));
                        ok = true;
                    }
                } catch (Exception ignored) {
                    // reported to the page as a failure
                }
            }
            pendingSave = null;
            callPage("VNShelfFileSaved", ok ? "true" : "false");
        } else if (request == REQ_OPEN_FILE) {
            String text = null;
            if (uri != null) {
                try (InputStream in = getContentResolver().openInputStream(uri)) {
                    if (in != null) {
                        ByteArrayOutputStream buf = new ByteArrayOutputStream();
                        byte[] chunk = new byte[16384];
                        int n;
                        while ((n = in.read(chunk)) > 0 && buf.size() <= MAX_OPEN_BYTES) {
                            buf.write(chunk, 0, n);
                        }
                        if (buf.size() <= MAX_OPEN_BYTES) {
                            text = new String(buf.toByteArray(), StandardCharsets.UTF_8);
                        }
                    }
                } catch (Exception ignored) {
                    // reported to the page as "nothing opened"
                }
            }
            callPage("VNShelfFileOpened", text == null ? "null" : JSONObject.quote(text));
        }
    }

    /** Call window.fn(arg) in the page if it defined it; arg is a JavaScript literal. */
    private void callPage(String fn, String arg) {
        if (web != null) {
            web.evaluateJavascript("window." + fn + " && window." + fn + "(" + arg + ")", null);
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
