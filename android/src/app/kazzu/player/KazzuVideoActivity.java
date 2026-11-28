package app.kazzu.player;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

import java.util.Collections;

/** Uses YouTube's own player for videos that allow embedding. */
public final class KazzuVideoActivity extends Activity {
    private WebView player;
    private FrameLayout root;
    private LinearLayout page;
    private View fullscreen;
    private WebChromeClient.CustomViewCallback fullscreenCallback;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        String video = getIntent().getStringExtra("video");
        if (video == null || !video.matches("[A-Za-z0-9_-]{1,32}")) {
            finish();
            return;
        }

        String videoPath = "/embed/" + video;
        root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        page.setGravity(Gravity.CENTER_HORIZONTAL);
        root.addView(page, new FrameLayout.LayoutParams(-1, -1));
        setContentView(root);

        LinearLayout toolbar = new LinearLayout(this);
        Button back = new Button(this);
        back.setText("Voltar");
        back.setOnClickListener(view -> finish());
        toolbar.addView(back);
        TextView title = new TextView(this);
        title.setText("Vídeo do YouTube");
        title.setTextColor(Color.WHITE);
        title.setGravity(Gravity.CENTER_VERTICAL);
        toolbar.addView(title, new LinearLayout.LayoutParams(0, -1, 1));
        Button external = new Button(this);
        external.setText("Abrir no YouTube");
        external.setOnClickListener(view -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse("https://www.youtube.com/watch?v=" + video)));
            } catch (android.content.ActivityNotFoundException ignored) {
                android.widget.Toast.makeText(this, "Nenhum aplicativo disponível para abrir o YouTube.", android.widget.Toast.LENGTH_SHORT).show();
            }
        });
        toolbar.addView(external);
        page.addView(toolbar, new LinearLayout.LayoutParams(-1, -2));

        try {
            player = new WebView(this);
        } catch (RuntimeException error) {
            TextView unavailable = new TextView(this);
            unavailable.setText("Reprodutor incorporado indisponível. Use Abrir no YouTube.");
            unavailable.setTextColor(Color.WHITE);
            page.addView(unavailable);
            return;
        }
        player.setBackgroundColor(Color.BLACK);
        player.getSettings().setJavaScriptEnabled(true);
        player.getSettings().setDomStorageEnabled(true);
        player.getSettings().setAllowFileAccess(false);
        player.getSettings().setAllowContentAccess(false);
        player.getSettings().setSupportMultipleWindows(false);
        player.getSettings().setMediaPlaybackRequiresUserGesture(true);
        android.webkit.CookieManager.getInstance().setAcceptThirdPartyCookies(player, false);
        player.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                if (!request.isForMainFrame()) return false;
                Uri url = request.getUrl();
                return !"https".equals(url.getScheme()) || !"www.youtube.com".equals(url.getHost())
                        || !videoPath.equals(url.getPath());
            }
        });
        player.setWebChromeClient(new WebChromeClient() {
            @Override public void onShowCustomView(View view, CustomViewCallback callback) {
                if (fullscreen != null) {
                    callback.onCustomViewHidden();
                    return;
                }
                fullscreen = view;
                fullscreenCallback = callback;
                page.setVisibility(View.INVISIBLE);
                root.addView(view, new FrameLayout.LayoutParams(-1, -1));
            }

            @Override public void onHideCustomView() { hideFullscreen(); }
        });
        page.addView(player, new LinearLayout.LayoutParams(-1, 0, 1));
        TextView privacy = new TextView(this);
        privacy.setText("YouTube · Política de privacidade");
        privacy.setTextColor(Color.WHITE);
        privacy.setPadding(12, 8, 12, 8);
        privacy.setOnClickListener(view -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse("https://policies.google.com/privacy")));
            } catch (android.content.ActivityNotFoundException ignored) {}
        });
        page.addView(privacy);
        // YouTube requires the operating-system package as Referer in WebView.
        player.loadUrl("https://www.youtube.com" + videoPath + "?playsinline=1",
                Collections.singletonMap("Referer", "https://" + getPackageName() + "/"));
    }

    private void hideFullscreen() {
        if (fullscreen == null) return;
        root.removeView(fullscreen);
        fullscreen = null;
        page.setVisibility(View.VISIBLE);
        WebChromeClient.CustomViewCallback callback = fullscreenCallback;
        fullscreenCallback = null;
        if (callback != null) callback.onCustomViewHidden();
    }

    @Override public void onBackPressed() {
        if (fullscreen != null) hideFullscreen();
        else super.onBackPressed();
    }

    @Override protected void onPause() {
        if (player != null) {
            player.evaluateJavascript("document.querySelectorAll('video').forEach(function(v) { v.pause(); });", null);
            player.onPause();
        }
        super.onPause();
    }

    @Override protected void onResume() {
        super.onResume();
        if (player != null) player.onResume();
    }

    @Override protected void onDestroy() {
        hideFullscreen();
        if (player != null) {
            page.removeView(player);
            player.stopLoading();
            player.destroy();
            player = null;
        }
        super.onDestroy();
    }
}
