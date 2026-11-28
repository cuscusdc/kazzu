package app.kazzu.player;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ServiceInfo;
import android.media.AudioManager;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.IBinder;
import android.os.SystemClock;

public final class KazzuMediaService extends Service {
    private static final String CHANNEL = "playback";
    private static final int NOTIFICATION = 1;
    // All accesses occur on the Android main looper.
    private static KazzuMediaService active;

    private MediaSession session;
    private NotificationManager notifications;
    private Playback playback;
    private boolean foreground;
    private final BroadcastReceiver noisy = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            if (AudioManager.ACTION_AUDIO_BECOMING_NOISY.equals(intent.getAction())) {
                KazzuActivity.nativeMediaAction("pause", 0);
            }
        }
    };

    static final class Playback {
        final String title;
        final String artist;
        final long duration;
        final long position;
        final boolean playing;
        final boolean buffering;

        Playback(String title, String artist, long duration, long position, boolean playing, boolean buffering) {
            this.title = title;
            this.artist = artist;
            this.duration = duration;
            this.position = position;
            this.playing = playing;
            this.buffering = buffering;
        }

        void writeTo(Intent intent) {
            intent.putExtra("title", title).putExtra("artist", artist)
                    .putExtra("duration", duration).putExtra("position", position)
                    .putExtra("playing", playing).putExtra("buffering", buffering);
        }

        static Playback readFrom(Intent intent) {
            String title = intent.getStringExtra("title");
            String artist = intent.getStringExtra("artist");
            return new Playback(title == null ? "" : title, artist == null ? "" : artist,
                    intent.getLongExtra("duration", 0), intent.getLongExtra("position", 0),
                    intent.getBooleanExtra("playing", false), intent.getBooleanExtra("buffering", false));
        }
    }

    static boolean publish(Playback playback) {
        if (active == null || active.playback == null) return false;
        active.update(playback);
        return true;
    }

    static boolean playingInBackground() {
        return active != null && active.foreground && active.playback != null
                && (active.playback.playing || active.playback.buffering);
    }

    @Override public void onCreate() {
        super.onCreate();
        active = this;
        notifications = getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(new NotificationChannel(
                CHANNEL, "Reprodução", NotificationManager.IMPORTANCE_LOW));

        session = new MediaSession(this, "Kazzu");
        session.setSessionActivity(activityIntent());
        session.setFlags(MediaSession.FLAG_HANDLES_MEDIA_BUTTONS | MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS);
        session.setCallback(new MediaSession.Callback() {
            @Override public void onPlay() { KazzuActivity.nativeMediaAction("play", 0); }
            @Override public void onPause() { KazzuActivity.nativeMediaAction("pause", 0); }
            @Override public void onSkipToNext() { KazzuActivity.nativeMediaAction("next", 0); }
            @Override public void onSkipToPrevious() { KazzuActivity.nativeMediaAction("previous", 0); }
            @Override public void onSeekTo(long position) { KazzuActivity.nativeMediaAction("seek", position); }
            @Override public void onStop() { KazzuActivity.nativeMediaAction("stop", 0); }
        });
        IntentFilter filter = new IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY);
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(noisy, filter, Context.RECEIVER_NOT_EXPORTED);
        else registerReceiver(noisy, filter);
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null) {
            stopSelf(startId);
        } else if (intent.getAction() == null) {
            update(Playback.readFrom(intent));
        } else if (playback != null) {
            KazzuActivity.nativeMediaAction(intent.getAction(), 0);
        }
        return START_NOT_STICKY;
    }

    private void update(Playback next) {
        Playback prior = playback;
        playback = next;
        if (next.title.isEmpty()) {
            session.setActive(false);
            stopForeground(STOP_FOREGROUND_REMOVE);
            foreground = false;
            stopSelf();
            return;
        }

        boolean metadataChanged = prior == null || !next.title.equals(prior.title)
                || !next.artist.equals(prior.artist) || next.duration != prior.duration;
        if (metadataChanged) {
            session.setMetadata(new MediaMetadata.Builder()
                    .putString(MediaMetadata.METADATA_KEY_TITLE, next.title)
                    .putString(MediaMetadata.METADATA_KEY_ARTIST, next.artist)
                    .putLong(MediaMetadata.METADATA_KEY_DURATION, next.duration).build());
        }
        int state = next.buffering ? PlaybackState.STATE_BUFFERING
                : next.playing ? PlaybackState.STATE_PLAYING : PlaybackState.STATE_PAUSED;
        long actions = PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE
                | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_SKIP_TO_NEXT
                | PlaybackState.ACTION_SKIP_TO_PREVIOUS | PlaybackState.ACTION_STOP;
        if (next.duration > 0) actions |= PlaybackState.ACTION_SEEK_TO;
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(actions)
                .setState(state, next.position, next.playing ? 1f : 0f, SystemClock.elapsedRealtime())
                .build());
        session.setActive(true);

        if ((next.playing || next.buffering) && !foreground) {
            try {
                Notification notice = notification(next);
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(NOTIFICATION, notice, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                } else {
                    startForeground(NOTIFICATION, notice);
                }
                foreground = true;
            } catch (IllegalStateException | SecurityException error) {
                KazzuActivity.nativeMediaAction("background-unavailable", 0);
                stopSelf();
            }
        } else if (metadataChanged || prior.playing != next.playing || prior.buffering != next.buffering) {
            notifications.notify(NOTIFICATION, notification(next));
        }
        if (foreground && !next.playing && !next.buffering) {
            stopForeground(STOP_FOREGROUND_DETACH);
            foreground = false;
        }
    }

    private PendingIntent activityIntent() {
        return PendingIntent.getActivity(this, 0, new Intent(this, KazzuActivity.class),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    private PendingIntent control(String action, int request) {
        return PendingIntent.getService(this, request, new Intent(this, KazzuMediaService.class).setAction(action),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    private Notification notification(Playback playback) {
        boolean playing = playback.playing || playback.buffering;
        return new Notification.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle(playback.title).setContentText(playback.artist)
                .setContentIntent(activityIntent()).setOnlyAlertOnce(true)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setStyle(new Notification.MediaStyle().setMediaSession(session.getSessionToken())
                        .setShowActionsInCompactView(0, 1, 2))
                .addAction(new Notification.Action.Builder(android.R.drawable.ic_media_previous,
                        "Anterior", control("previous", 1)).build())
                .addAction(new Notification.Action.Builder(playing ? android.R.drawable.ic_media_pause
                        : android.R.drawable.ic_media_play, playing ? "Pausar" : "Tocar",
                        control(playing ? "pause" : "play", 2)).build())
                .addAction(new Notification.Action.Builder(android.R.drawable.ic_media_next,
                        "Próxima", control("next", 3)).build())
                .build();
    }

    @Override public void onDestroy() {
        active = null;
        unregisterReceiver(noisy);
        session.release();
        notifications.cancel(NOTIFICATION);
        super.onDestroy();
    }

    @Override public IBinder onBind(Intent intent) { return null; }
}
