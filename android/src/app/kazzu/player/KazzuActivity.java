package app.kazzu.player;

import android.app.NativeActivity;
import android.content.Intent;
import android.content.UriPermission;
import android.graphics.Rect;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.util.DisplayMetrics;
import android.view.View;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import java.io.IOException;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import android.database.Cursor;
import android.provider.OpenableColumns;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.time.DayOfWeek;
import java.time.LocalDate;
import java.time.ZoneId;
import java.time.temporal.TemporalAdjusters;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONArray;
import org.json.JSONObject;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;

public final class KazzuActivity extends NativeActivity {
    static { System.loadLibrary("kazzu_app"); }

    private static final int OPEN_MUSIC_TREE = 101;
    private static final int CREATE_STATS_DOCUMENT = 102;
    private static final int OPEN_M3U = 103;
    private static final int CREATE_BACKUP_DOCUMENT = 104;
    private static final int OPEN_BACKUP_DOCUMENT = 105;
    private static final int OPEN_PLAYLIST_COVER = 106;
    private String coverPlaylist;
    private String documentExport;
    private final ExecutorService imports = Executors.newSingleThreadExecutor();
    private final ExecutorService remote = Executors.newSingleThreadExecutor();
    private final ExecutorService social = Executors.newSingleThreadExecutor();
    private LastFm lastFm;
    private volatile boolean lastFmEnabled;
    private boolean lastFmFailureReported;
    private LanPlayback remoteHost;
    private String followCode;
    private int playlistSyncTick;
    private volatile byte[] roomBackup;
    private volatile LanPlayback.Now roomPlayback;
    private boolean syncing;
    private boolean rescanRequested;
    private boolean serviceStarting;
    private final Handler main = new Handler(Looper.getMainLooper());
    private AudioFocusRequest focusRequest;
    private View accessibilityHost;
    private KazzuAccessibilityProvider accessibilityProvider;
    private View.AccessibilityDelegate accessibilityDelegate;
    private String pendingAccessibilityTree = "{\"nodes\":[]}";

    static native void nativeRoomPlaylistTrack(String playlist, String id, String title, String artist);
    static native void nativeFollowTrack(String id, String title, String artist, long position, boolean playing);
    static native void nativePlaylistCover(String id, byte[] image);
    static native void nativeRemoteCode(String code);
    static native void nativeRestoreBackup(String json);
    static native void nativeSyncPeerPlaylists(String json);
    static native void nativeImportM3u(String title, String contents);
    static native void nativeMediaAction(String action, long positionMs);
    static native void nativeShutdown();

    public void publishPlayback(String id, String title, String artist, long durationMs,
            long positionMs, boolean playing, boolean buffering) {
        main.post(() -> {
            if (isDestroyed()) return;
            if (id.startsWith("yt:") && id.substring(3).matches("[A-Za-z0-9_-]{1,32}")) {
                roomPlayback = new LanPlayback.Now(id.substring(3), title, artist, positionMs, playing && !buffering);
            } else {
                roomPlayback = null;
            }
            KazzuMediaService.Playback update = new KazzuMediaService.Playback(
                    title, artist, durationMs, positionMs, playing, buffering);
            if (KazzuMediaService.publish(update)) {
                serviceStarting = false;
                return;
            }
            if (serviceStarting || title.isEmpty() || (!playing && !buffering)) return;
            Intent intent = new Intent(this, KazzuMediaService.class);
            update.writeTo(intent);
            try {
                serviceStarting = true;
                startForegroundService(intent);
            } catch (IllegalStateException | SecurityException error) {
                serviceStarting = false;
                nativeMediaAction("background-unavailable", 0);
            }
        });
    }

    public boolean prefersReducedMotion() {
        return android.provider.Settings.Global.getFloat(getContentResolver(),
                android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f;
    }

    public void updateAccessibilityTree(String json) {
        pendingAccessibilityTree = json == null ? "{\"nodes\":[]}" : json;
        main.post(() -> {
            installAccessibility();
            if (accessibilityProvider != null) accessibilityProvider.replace(pendingAccessibilityTree);
        });
    }

    private void installAccessibility() {
        if (accessibilityProvider != null && accessibilityHost != null) return;
        if (getWindow() == null) return;
        View decor = getWindow().getDecorView();
        if (decor == null) return;
        accessibilityHost = decor;
        accessibilityProvider = new KazzuAccessibilityProvider();
        accessibilityDelegate = new View.AccessibilityDelegate() {
            @Override public AccessibilityNodeProvider getAccessibilityNodeProvider(View host) {
                return accessibilityProvider;
            }
        };
        decor.setAccessibilityDelegate(accessibilityDelegate);
        decor.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        accessibilityProvider.replace(pendingAccessibilityTree);
    }

    public synchronized boolean requestPlaybackFocus() {
        AudioManager manager = getSystemService(AudioManager.class);
        if (manager == null) return false;
        if (focusRequest == null) {
            focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                            .build())
                    .setWillPauseWhenDucked(true)
                    .setOnAudioFocusChangeListener(change -> {
                        if (change < 0) nativeMediaAction("pause", 0);
                    }, main)
                    .build();
        }
        return manager.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
    }

    private synchronized void abandonPlaybackFocus() {
        AudioManager manager = getSystemService(AudioManager.class);
        if (manager != null && focusRequest != null) manager.abandonAudioFocusRequest(focusRequest);
        focusRequest = null;
    }

    public long listeningPeriodStart(int period) {
        LocalDate today = LocalDate.now();
        LocalDate start;
        switch (period) {
            case 0: start = today; break;
            case 1: start = today.with(TemporalAdjusters.previousOrSame(DayOfWeek.MONDAY)); break;
            case 2: start = today.withDayOfMonth(1); break;
            case 3: start = today.withDayOfYear(1); break;
            default: return 0;
        }
        return start.atStartOfDay(ZoneId.systemDefault()).toEpochSecond();
    }

    public void exportListeningStats(String json) {
        exportDocument(json, "kazzu-estatisticas.json", CREATE_STATS_DOCUMENT);
    }

    public void exportBackup(String json) {
        exportDocument(json, "kazzu-backup.json", CREATE_BACKUP_DOCUMENT);
    }

    private void exportDocument(String json, String filename, int request) {
        main.post(() -> {
            if (documentExport != null || isDestroyed()) return;
            documentExport = json;
            try {
                startActivityForResult(new Intent(Intent.ACTION_CREATE_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
                        .putExtra(Intent.EXTRA_TITLE, filename), request);
            } catch (android.content.ActivityNotFoundException error) {
                documentExport = null;
                nativeMediaAction(request == CREATE_BACKUP_DOCUMENT ? "backup-export-failed" : "stats-export-failed", 0);
            }
        });
    }

    public void selectPlaylistCover(String id) {
        main.post(() -> {
            if (isDestroyed() || coverPlaylist != null) return;
            coverPlaylist = id;
            try {
                startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE).setType("image/*"), OPEN_PLAYLIST_COVER);
            } catch (android.content.ActivityNotFoundException error) {
                coverPlaylist = null;
                nativeMediaAction("cover-read-failed", 0);
            }
        });
    }

    private void readPlaylistCover(String id, Uri input) {
        try (InputStream source = getContentResolver().openInputStream(input);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (source == null) throw new IOException("Imagem indisponível");
            byte[] buffer = new byte[8192];
            int count;
            while ((count = source.read(buffer)) != -1) {
                if (output.size() + count > 8 * 1024 * 1024) throw new IOException("Imagem excede o limite");
                output.write(buffer, 0, count);
            }
            if (!isDestroyed()) nativePlaylistCover(id, output.toByteArray());
        } catch (IOException | RuntimeException error) {
            nativeMediaAction("cover-read-failed", 0);
        }
    }

    public void selectBackup() {
        main.post(() -> {
            try {
                startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE).setType("application/json"), OPEN_BACKUP_DOCUMENT);
            } catch (android.content.ActivityNotFoundException error) {
                nativeMediaAction("backup-read-failed", 0);
            }
        });
    }

    private void readBackup(Uri input) {
        try (InputStream source = getContentResolver().openInputStream(input);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (source == null) throw new IOException("Arquivo indisponível");
            byte[] buffer = new byte[8192];
            int count;
            while ((count = source.read(buffer)) != -1) {
                if (output.size() + count > 8 * 1024 * 1024) throw new IOException("Backup excede o limite");
                output.write(buffer, 0, count);
            }
            if (!isDestroyed()) nativeRestoreBackup(new String(output.toByteArray(), StandardCharsets.UTF_8));
        } catch (IOException | RuntimeException error) {
            nativeMediaAction("backup-read-failed", 0);
        }
    }

    public void openVideo(String video) {
        main.post(() -> {
            if (!isDestroyed()) {
                startActivity(new Intent(this, KazzuVideoActivity.class).putExtra("video", video));
            }
        });
    }

    public void shareTrack(String text) {
        main.post(() -> {
            Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text);
            try {
                startActivity(Intent.createChooser(send, "Compartilhar música"));
            } catch (android.content.ActivityNotFoundException error) {
                nativeMediaAction("share-unavailable", 0);
            }
        });
    }

    public void selectM3u() {
        main.post(() -> {
            try {
                startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE).setType("*/*"), OPEN_M3U);
            } catch (android.content.ActivityNotFoundException error) {
                nativeMediaAction("playlist-import-failed", 0);
            }
        });
    }

    private void readM3u(Uri input) {
        try {
            String name;
            try (Cursor result = getContentResolver().query(input,
                    new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
                if (result == null || !result.moveToFirst()) throw new IOException("Nome indisponível");
                name = result.getString(0);
            }
            String lower = name == null ? "" : name.toLowerCase(java.util.Locale.ROOT);
            if (!lower.endsWith(".m3u") && !lower.endsWith(".m3u8")) throw new IOException("Formato inválido");
            try (InputStream source = getContentResolver().openInputStream(input);
                 ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                if (source == null) throw new IOException("Arquivo indisponível");
                byte[] buffer = new byte[8192];
                int count;
                while ((count = source.read(buffer)) != -1) {
                    if (output.size() + count > 1024 * 1024) throw new IOException("Arquivo excede o limite");
                    output.write(buffer, 0, count);
                }
                String title = name.substring(0, Math.min(255, name.lastIndexOf('.')));
                if (!isDestroyed()) nativeImportM3u(title, new String(output.toByteArray(), StandardCharsets.UTF_8));
            }
        } catch (IOException | RuntimeException error) {
            nativeMediaAction("playlist-import-failed", 0);
        }
    }

    public void selectMusicFolder() {
        main.post(() -> {
            Intent picker = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            try {
                startActivityForResult(picker, OPEN_MUSIC_TREE);
            } catch (android.content.ActivityNotFoundException error) {
                nativeMediaAction("folder-unavailable", 0);
            }
        });
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == OPEN_M3U) {
            Uri input = data == null ? null : data.getData();
            if (result == RESULT_OK && input != null) imports.execute(() -> readM3u(input));
            return;
        }
        if (request == OPEN_PLAYLIST_COVER) {
            String id = coverPlaylist;
            coverPlaylist = null;
            Uri input = data == null ? null : data.getData();
            if (result == RESULT_OK && input != null && id != null) {
                imports.execute(() -> readPlaylistCover(id, input));
            }
            return;
        }
        if (request == OPEN_BACKUP_DOCUMENT) {
            Uri input = data == null ? null : data.getData();
            if (result == RESULT_OK && input != null) imports.execute(() -> readBackup(input));
            return;
        }
        if (request == CREATE_STATS_DOCUMENT || request == CREATE_BACKUP_DOCUMENT) {
            boolean backup = request == CREATE_BACKUP_DOCUMENT;
            String json = documentExport;
            documentExport = null;
            Uri output = data == null ? null : data.getData();
            if (result == RESULT_OK && output != null && json != null) {
                imports.execute(() -> {
                    try (OutputStream file = getContentResolver().openOutputStream(output, "wt")) {
                        if (file == null) throw new IOException("Arquivo indisponível");
                        file.write(json.getBytes(StandardCharsets.UTF_8));
                    } catch (IOException | SecurityException error) {
                        nativeMediaAction(backup ? "backup-export-failed" : "stats-export-failed", 0);
                        return;
                    }
                    nativeMediaAction(backup ? "backup-exported" : "stats-exported", 0);
                });
            }
            return;
        }
        if (request != OPEN_MUSIC_TREE || result != RESULT_OK || data == null) return;
        Uri tree = data.getData();
        if (tree == null || !DocumentsContract.isTreeUri(tree)) return;
        try {
            getContentResolver().takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION);
            refreshMusicFolders();
        } catch (SecurityException error) {
            nativeMediaAction("folder-permission", 0);
        }
    }

    @Override protected void onPostResume() {
        super.onPostResume();
        installAccessibility();
        refreshMusicFolders();
    }

    private LastFm lastFm() {
        if (lastFm == null) lastFm = new LastFm(getApplicationContext());
        return lastFm;
    }

    public void lastFmStatus() {
        social.execute(() -> {
            try {
                lastFmEnabled = lastFm().configured();
                nativeMediaAction(lastFmEnabled ? "lastfm-connected" : "lastfm-disconnected", 0);
            } catch (Exception error) {
                nativeMediaAction("lastfm-authorize-failed", 0);
            }
        });
    }

    public void lastFmAuthorize(String apiKey, String secret) {
        social.execute(() -> {
            try {
                String url = lastFm().authorize(apiKey, secret);
                main.post(() -> {
                    if (isDestroyed()) return;
                    try {
                        startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
                        nativeMediaAction("lastfm-confirm", 0);
                    } catch (android.content.ActivityNotFoundException error) {
                        nativeMediaAction("lastfm-authorize-failed", 0);
                    }
                });
            } catch (Exception error) {
                nativeMediaAction("lastfm-authorize-failed", 0);
            }
        });
    }

    public void lastFmFinish(boolean connect) {
        if (!connect) lastFmEnabled = false;
        social.execute(() -> {
            try {
                if (connect) {
                    lastFm().finishAuthorization();
                    lastFmEnabled = true;
                } else {
                    lastFm().disconnect();
                }
                lastFmFailureReported = false;
                nativeMediaAction(connect ? "lastfm-connected" : "lastfm-disconnected", 0);
            } catch (Exception error) {
                nativeMediaAction("lastfm-authorize-failed", 0);
            }
        });
    }

    public void lastFmScrobble(String title, String artist, String album, long started, long duration) {
        if (!lastFmEnabled || social.isShutdown()) return;
        social.execute(() -> {
            if (!lastFmEnabled) return;
            try {
                lastFm().scrobble(title, artist, album, started, duration);
                lastFmFailureReported = false;
            } catch (Exception error) {
                if (!lastFmFailureReported) nativeMediaAction("lastfm-scrobble-failed", 0);
                lastFmFailureReported = true;
            }
        });
    }

    public void hostRemote(boolean enabled, boolean share, String playlist) {
        main.post(() -> {
            if (isDestroyed()) return;
            if (!enabled) {
                closeRemote();
                nativeRemoteCode("");
                return;
            }
            closeRemote();
            try {
                remoteHost = new LanPlayback(command -> {
                    String[] actions = { "toggle", "previous", "next", "pause" };
                    nativeMediaAction(actions[command], 0);
                }, () -> share ? roomPlayback : null, playlist.isEmpty() ? null : track ->
                    nativeRoomPlaylistTrack(playlist, track.id, track.title, track.artist), () -> share ? roomBackup : null, host -> main.post(() -> {
                    if (remoteHost == host) {
                        closeRemote();
                        if (!isDestroyed()) nativeRemoteCode("");
                    }
                }));
                nativeRemoteCode(remoteHost.code());
            } catch (IOException error) {
                nativeRemoteCode("");
                nativeMediaAction("remote-host-failed", 0);
            }
        });
    }

    public void followRoom(String code) {
        main.post(() -> {
            if (isDestroyed()) return;
            followCode = code.isEmpty() ? null : code;
            playlistSyncTick = 0;
            nativeMediaAction(followCode == null ? "follow-stopped" : "follow-started", 0);
            if (followCode != null) {
                syncPeerPlaylists(followCode);
                pollRoom(followCode);
            }
        });
    }

    private void pollRoom(String code) {
        if (isDestroyed() || followCode != code) return;
        remote.execute(() -> {
            try {
                LanPlayback.Now now = LanPlayback.sync(code);
                main.post(() -> {
                    if (isDestroyed() || followCode != code) return;
                    if (now == null) {
                        nativeMediaAction("follow-waiting", 0);
                    } else {
                        nativeFollowTrack(now.id, now.title, now.artist, now.currentPosition(), now.playing);
                    }
                    if (++playlistSyncTick % 4 == 0) syncPeerPlaylists(code);
                    main.postDelayed(() -> pollRoom(code), 3000);
                });
            } catch (IOException | java.security.GeneralSecurityException error) {
                main.post(() -> {
                    if (followCode == code) {
                        followCode = null;
                        nativeMediaAction("follow-failed", 0);
                    }
                });
            }
        });
    }

    public void hostBackup(String json) {
        main.post(() -> {
            if (isDestroyed()) return;
            closeRemote();
            try {
            roomBackup = json.getBytes(StandardCharsets.UTF_8);
            remoteHost = new LanPlayback(null, () -> null, null, () -> roomBackup, host -> main.post(() -> {
                    if (remoteHost == host) {
                        closeRemote();
                        if (!isDestroyed()) nativeRemoteCode("");
                    }
                }));
                nativeRemoteCode(remoteHost.code());
            } catch (IOException error) {
                nativeRemoteCode("");
                nativeMediaAction("remote-host-failed", 0);
            }
        });
    }

    public void receiveRoomBackup(String json) {
        main.post(() -> {
            if (!isDestroyed()) roomBackup = json.getBytes(StandardCharsets.UTF_8);
        });
    }

    private void syncPeerPlaylists(String code) {
        remote.execute(() -> {
            try {
                String backup = LanPlayback.copyBackup(code);
                if (!isDestroyed()) nativeSyncPeerPlaylists(backup);
            } catch (IOException | java.security.GeneralSecurityException ignored) {
            }
        });
    }

    public void copyBackupFromPeer(String code) {
        main.post(() -> {
            if (isDestroyed()) return;
            remote.execute(() -> {
                try {
                    String backup = LanPlayback.copyBackup(code);
                    if (!isDestroyed()) nativeRestoreBackup(backup);
                } catch (IOException | java.security.GeneralSecurityException error) {
                    nativeMediaAction("peer-backup-failed", 0);
                }
            });
        });
    }

    public void suggestCurrentTrack(String code) {
        LanPlayback.Now track = roomPlayback;
        if (track == null) {
            nativeMediaAction("suggest-no-track", 0);
            return;
        }
        main.post(() -> {
            if (isDestroyed()) return;
            remote.execute(() -> {
                try {
                    boolean accepted = LanPlayback.suggest(code, track);
                    nativeMediaAction(accepted ? "suggest-received" : "suggest-unavailable", 0);
                } catch (IOException | java.security.GeneralSecurityException error) {
                    nativeMediaAction("remote-send-failed", 0);
                }
            });
        });
    }

    public void sendRemote(String code, int command) {
        main.post(() -> {
            if (isDestroyed()) return;
            remote.execute(() -> {
                try {
                    LanPlayback.send(code, command);
                    nativeMediaAction("remote-sent", 0);
                } catch (IOException | java.security.GeneralSecurityException error) {
                    nativeMediaAction("remote-send-failed", 0);
                }
            });
        });
    }

    private void closeRemote() {
        if (remoteHost != null) remoteHost.close();
        remoteHost = null;
    }

    public void findDuplicates() {
        main.post(() -> {
            if (isDestroyed()) return;
            imports.execute(() -> {
                try {
                    nativeMediaAction("duplicates-found", LocalMusicImport.duplicateCopies(this));
                } catch (IOException | RuntimeException error) {
                    nativeMediaAction("duplicates-failed", 0);
                }
            });
        });
    }

    public void refreshMusicFolders() {
        main.post(() -> {
            if (isDestroyed()) return;
            if (syncing) {
                rescanRequested = true;
                return;
            }
            syncing = true;
            imports.execute(() -> {
                boolean changed = false;
                boolean failed = false;
                for (UriPermission permission : getContentResolver().getPersistedUriPermissions()) {
                    if (!permission.isReadPermission() || !DocumentsContract.isTreeUri(permission.getUri())) continue;
                    try {
                        changed |= LocalMusicImport.mirror(this, permission.getUri());
                    } catch (IOException | RuntimeException error) {
                        failed = true;
                    }
                }
                final boolean any = changed;
                final boolean errors = failed;
                main.post(() -> {
                    syncing = false;
                    if (isDestroyed()) return;
                    if (any) nativeMediaAction("folder-imported", 0);
                    if (errors) nativeMediaAction("folder-import-failed", 0);
                    if (rescanRequested) {
                        rescanRequested = false;
                        refreshMusicFolders();
                    }
                });
            });
        });
    }

    @Override public void onBackPressed() {
        if (KazzuMediaService.playingInBackground()) {
            moveTaskToBack(true);
        } else {
            super.onBackPressed();
        }
    }

    @Override protected void onDestroy() {
        boolean recreating = isChangingConfigurations();
        closeRemote();
        followCode = null;
        if (!recreating) {
            remote.shutdownNow();
            social.shutdown();
            imports.shutdownNow();
            abandonPlaybackFocus();
            if (accessibilityHost != null) {
                accessibilityProvider = null;
                accessibilityHost.setAccessibilityDelegate(null);
                accessibilityDelegate = null;
            }
            nativeShutdown();
            stopService(new Intent(this, KazzuMediaService.class));
        }
        super.onDestroy();
    }

    private static final class AccessibilityNode {
        String id;
        String role;
        String label;
        String description;
        String parent;
        String value;
        String click;
        String increment;
        String decrement;
        String delete;
        Boolean checked;
        final List<String> children = new ArrayList<>();
    }

    private final class KazzuAccessibilityProvider extends AccessibilityNodeProvider {
        private static final int ACTION_INCREMENT = 0x01000001;
        private static final int ACTION_DECREMENT = 0x01000002;
        private static final int ACTION_DELETE = 0x01000003;
        private final AccessibilityNodeInfo.AccessibilityAction incrementAction =
                new AccessibilityNodeInfo.AccessibilityAction(ACTION_INCREMENT, "Aumentar");
        private final AccessibilityNodeInfo.AccessibilityAction decrementAction =
                new AccessibilityNodeInfo.AccessibilityAction(ACTION_DECREMENT, "Diminuir");
        private final AccessibilityNodeInfo.AccessibilityAction deleteAction =
                new AccessibilityNodeInfo.AccessibilityAction(ACTION_DELETE, "Remover");
        private final Map<String, Integer> ids = new HashMap<>();
        private final Map<Integer, AccessibilityNode> nodes = new HashMap<>();
        private final List<Integer> order = new ArrayList<>();
        private int nextId = 1;

        private synchronized void replace(String json) {
            Map<Integer, AccessibilityNode> nextNodes = new HashMap<>();
            List<Integer> nextOrder = new ArrayList<>();
            try {
                JSONArray array = new JSONObject(json).optJSONArray("nodes");
                if (array == null) return;
                for (int i = 0; i < array.length(); i++) {
                    JSONObject item = array.getJSONObject(i);
                    String key = item.optString("id", "");
                    if (key.isEmpty()) continue;
                    Integer existing = ids.get(key);
                    int virtualId = existing == null ? nextId++ : existing;
                    ids.put(key, virtualId);
                    AccessibilityNode node = new AccessibilityNode();
                    node.id = key;
                    node.role = item.optString("role", "text");
                    node.label = item.optString("label", "");
                    node.description = item.optString("description", "");
                    node.parent = item.optString("parent", "");
                    node.value = item.optString("value", "");
                    node.click = item.optString("click", "");
                    node.increment = item.optString("increment", "");
                    node.decrement = item.optString("decrement", "");
                    node.delete = item.optString("delete", "");
                    if (item.has("checked") && !item.isNull("checked")) {
                        node.checked = item.optBoolean("checked", false);
                    }
                    JSONArray children = item.optJSONArray("children");
                    if (children != null) {
                        for (int child = 0; child < children.length(); child++) {
                            node.children.add(children.optString(child, ""));
                        }
                    }
                    nextNodes.put(virtualId, node);
                    if (node.parent.isEmpty()) nextOrder.add(0, virtualId);
                    else nextOrder.add(virtualId);
                }
            } catch (Exception error) {
                return;
            }
            nodes.clear();
            nodes.putAll(nextNodes);
            order.clear();
            order.addAll(nextOrder);
            announce();
        }

        private synchronized AccessibilityNode node(int virtualViewId) {
            return nodes.get(virtualViewId);
        }

        private void announce() {
            AccessibilityManager manager = (AccessibilityManager) getSystemService(ACCESSIBILITY_SERVICE);
            if (manager == null || !manager.isEnabled() || accessibilityHost == null) return;
            try {
                AccessibilityEvent event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED);
                event.setPackageName(getPackageName());
                event.setClassName(KazzuActivity.class.getName());
                event.setContentChangeTypes(AccessibilityEvent.CONTENT_CHANGE_TYPE_CONTENT_DESCRIPTION);
                event.setSource(accessibilityHost, HOST_VIEW_ID);
                manager.sendAccessibilityEvent(event);
            } catch (RuntimeException error) {
                return;
            }
        }

        private Rect bounds(int virtualViewId) {
            DisplayMetrics metrics = new DisplayMetrics();
            if (getWindowManager() != null) getWindowManager().getDefaultDisplay().getMetrics(metrics);
            int[] location = new int[2];
            if (accessibilityHost != null) accessibilityHost.getLocationOnScreen(location);
            int left = location[0] + 16;
            int right = location[0] + Math.max(17, metrics.widthPixels - 16);
            int top = location[1] + 8;
            int bottom = location[1] + Math.max(9, metrics.heightPixels - 8);
            if (virtualViewId == HOST_VIEW_ID || order.isEmpty()) return new Rect(left, top, right, bottom);
            int count = order.size();
            int slot = Math.max(1, (bottom - top) / count);
            int index = Math.max(0, order.indexOf(virtualViewId));
            int rowTop = top + index * slot;
            int rowBottom = Math.min(bottom, rowTop + Math.max(48, slot - 4));
            if (rowBottom <= rowTop) rowBottom = Math.min(bottom, rowTop + 48);
            return new Rect(left, rowTop, right, Math.max(rowTop + 1, rowBottom));
        }

        private String className(String role) {
            if ("button".equals(role)) return "android.widget.Button";
            if ("checkbox".equals(role)) return "android.widget.CheckBox";
            if ("slider".equals(role)) return "android.widget.SeekBar";
            if ("progressbar".equals(role)) return "android.widget.ProgressBar";
            if ("text".equals(role)) return "android.widget.TextView";
            if ("group".equals(role)) return "android.view.ViewGroup";
            return View.class.getName();
        }

        private AccessibilityNodeInfo rootInfo() {
            if (accessibilityHost == null) return null;
            AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(accessibilityHost);
            info.setPackageName(getPackageName());
            info.setClassName("android.view.ViewGroup");
            info.setContentDescription("Kazzu");
            info.setVisibleToUser(true);
            info.setFocusable(true);
            info.setBoundsInScreen(bounds(HOST_VIEW_ID));
            for (Integer id : order) {
                AccessibilityNode node = node(id);
                if (node != null && node.parent.isEmpty()) info.addChild(accessibilityHost, id);
            }
            return info;
        }

        @Override public AccessibilityNodeInfo createAccessibilityNodeInfo(int virtualViewId) {
            if (virtualViewId == HOST_VIEW_ID) return rootInfo();
            AccessibilityNode node = node(virtualViewId);
            if (node == null || accessibilityHost == null) return null;
            AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(accessibilityHost, virtualViewId);
            info.setPackageName(getPackageName());
            info.setClassName(className(node.role));
            String label = node.label.isEmpty() ? node.description : node.label;
            if (!node.value.isEmpty() && !label.contains(node.value)) {
                label = label.isEmpty() ? node.value : label + ", " + node.value;
            }
            info.setContentDescription(label);
            if (!node.description.isEmpty() && !node.description.equals(label)) info.setText(node.description);
            info.setEnabled(true);
            info.setVisibleToUser(true);
            info.setFocusable(true);
            if (node.checked != null) {
                info.setCheckable(true);
                info.setChecked(node.checked);
            }
            if (!node.click.isEmpty()) {
                info.setClickable(true);
                info.addAction(AccessibilityNodeInfo.ACTION_CLICK);
            }
            if (!node.increment.isEmpty()) info.addAction(incrementAction);
            if (!node.decrement.isEmpty()) info.addAction(decrementAction);
            if (!node.delete.isEmpty()) info.addAction(deleteAction);
            info.setBoundsInScreen(bounds(virtualViewId));
            for (String key : node.children) {
                Integer child = ids.get(key);
                if (child != null) info.addChild(accessibilityHost, child);
            }
            return info;
        }

        @Override public AccessibilityNodeInfo findFocus(int focusDirection) {
            if (order.isEmpty()) return null;
            int index = focusDirection == View.FOCUS_BACKWARD ? order.size() - 1 : 0;
            return createAccessibilityNodeInfo(order.get(index));
        }

        @Override public boolean performAction(int virtualViewId, int action, Bundle arguments) {
            AccessibilityNode node = node(virtualViewId);
            if (node == null) return false;
            String command = "";
            if (action == AccessibilityNodeInfo.ACTION_CLICK) command = node.click;
            else if (action == ACTION_INCREMENT) command = node.increment;
            else if (action == ACTION_DECREMENT) command = node.decrement;
            else if (action == ACTION_DELETE) command = node.delete;
            if (command.isEmpty()) return false;
            nativeMediaAction(command, 0);
            return true;
        }
    }
}
