package app.kazzu.player;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.IOException;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.util.Map;
import java.util.TreeMap;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import javax.net.ssl.HttpsURLConnection;

/** User-initiated Last.fm authorization and write-only scrobbling. */
final class LastFm {
    private static final String ALIAS = "kazzu.lastfm.v1";
    private static final int IV_LENGTH = 12;
    private final SharedPreferences prefs;
    private Credentials credentials;
    private String pendingKey;
    private String pendingSecret;
    private String pendingToken;

    LastFm(Context context) {
        prefs = context.getSharedPreferences("lastfm_private", Context.MODE_PRIVATE);
    }

    private static final class Credentials {
        final String apiKey;
        final String secret;
        final String session;
        Credentials(String apiKey, String secret, String session) {
            this.apiKey = apiKey;
            this.secret = secret;
            this.session = session;
        }
    }

    String authorize(String apiKey, String secret) throws Exception {
        if (!hexKey(apiKey) || !hexKey(secret)) throw new IOException("Credenciais da API inválidas");
        TreeMap<String, String> fields = fields("auth.getToken", apiKey);
        String token = post(fields, secret).getString("token");
        if (!hexKey(token)) throw new IOException("Token do Last.fm inválido");
        pendingKey = apiKey;
        pendingSecret = secret;
        pendingToken = token;
        return "https://www.last.fm/api/auth/?api_key=" + apiKey + "&token=" + token;
    }

    void finishAuthorization() throws Exception {
        if (pendingToken == null) throw new IOException("Inicie a autorização antes de confirmar");
        TreeMap<String, String> fields = fields("auth.getSession", pendingKey);
        fields.put("token", pendingToken);
        String session = post(fields, pendingSecret).getJSONObject("session").getString("key");
        if (!session.matches("[A-Za-z0-9_-]{16,128}")) throw new IOException("Sessão do Last.fm inválida");
        Credentials accepted = new Credentials(pendingKey, pendingSecret, session);
        save(accepted);
        credentials = accepted;
        clearPending();
    }

    boolean configured() throws Exception {
        load();
        return credentials != null;
    }

    void disconnect() {
        credentials = null;
        clearPending();
        prefs.edit().remove("sealed").apply();
    }

    private void clearPending() {
        pendingKey = null;
        pendingSecret = null;
        pendingToken = null;
    }

    void scrobble(String title, String artist, String album, long timestamp, long duration) throws Exception {
        load();
        if (credentials == null) return;
        TreeMap<String, String> fields = fields("track.scrobble", credentials.apiKey);
        fields.put("sk", credentials.session);
        fields.put("track", title);
        fields.put("artist", artist);
        if (!album.isEmpty()) fields.put("album", album);
        fields.put("timestamp", Long.toString(timestamp));
        fields.put("duration", Long.toString(duration));
        JSONObject response = post(fields, credentials.secret);
        if (response.getJSONObject("scrobbles").getJSONObject("@attr").optInt("accepted") != 1) {
            throw new IOException("O Last.fm não aceitou a reprodução");
        }
    }

    private static TreeMap<String, String> fields(String method, String apiKey) {
        TreeMap<String, String> fields = new TreeMap<>();
        fields.put("method", method);
        fields.put("api_key", apiKey);
        return fields;
    }

    private static boolean hexKey(String value) {
        return value != null && value.matches("[A-Fa-f0-9]{32}");
    }

    private static JSONObject post(TreeMap<String, String> fields, String secret) throws Exception {
        StringBuilder signed = new StringBuilder();
        for (Map.Entry<String, String> entry : fields.entrySet()) signed.append(entry.getKey()).append(entry.getValue());
        byte[] digest = MessageDigest.getInstance("MD5")
                .digest(signed.append(secret).toString().getBytes(StandardCharsets.UTF_8));
        StringBuilder signature = new StringBuilder(32);
        for (byte b : digest) signature.append(Character.forDigit((b >> 4) & 15, 16)).append(Character.forDigit(b & 15, 16));
        fields.put("api_sig", signature.toString());
        fields.put("format", "json");

        StringBuilder form = new StringBuilder();
        for (Map.Entry<String, String> entry : fields.entrySet()) {
            if (form.length() > 0) form.append('&');
            form.append(entry.getKey()).append('=').append(URLEncoder.encode(entry.getValue(), "UTF-8"));
        }
        byte[] body = form.toString().getBytes(StandardCharsets.UTF_8);
        HttpsURLConnection connection = (HttpsURLConnection) new URL("https://ws.audioscrobbler.com/2.0/").openConnection();
        connection.setConnectTimeout(8000);
        connection.setReadTimeout(8000);
        connection.setInstanceFollowRedirects(false);
        connection.setRequestMethod("POST");
        connection.setRequestProperty("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8");
        connection.setDoOutput(true);
        connection.setFixedLengthStreamingMode(body.length);
        try {
            try (OutputStream output = connection.getOutputStream()) { output.write(body); }
            if (connection.getResponseCode() != 200) throw new IOException("Falha ao contatar o Last.fm");
            try (InputStream input = connection.getInputStream(); ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                byte[] buffer = new byte[4096];
                int read;
                while ((read = input.read(buffer)) != -1) {
                    if (output.size() + read > 128 * 1024) throw new IOException("Resposta acima do limite");
                    output.write(buffer, 0, read);
                }
                JSONObject response = new JSONObject(new String(output.toByteArray(), StandardCharsets.UTF_8));
                if (response.has("error")) throw new IOException("O Last.fm recusou a operação");
                return response;
            }
        } finally {
            connection.disconnect();
        }
    }

    private static SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        java.security.Key existing = store.getKey(ALIAS, null);
        if (existing instanceof SecretKey) return (SecretKey) existing;
        if (!create) throw new IOException("Chave do Android indisponível");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }

    private void save(Credentials value) throws Exception {
        JSONObject json = new JSONObject().put("key", value.apiKey).put("secret", value.secret).put("session", value.session);
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, key(true));
        byte[] iv = cipher.getIV();
        byte[] encrypted = cipher.doFinal(json.toString().getBytes(StandardCharsets.UTF_8));
        if (iv.length != IV_LENGTH) throw new IOException("Formato criptográfico inválido");
        byte[] sealed = ByteBuffer.allocate(iv.length + encrypted.length).put(iv).put(encrypted).array();
        if (!prefs.edit().putString("sealed", Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
            throw new IOException("Não foi possível guardar a sessão");
        }
    }

    private void load() throws Exception {
        if (credentials != null) return;
        String encoded = prefs.getString("sealed", null);
        if (encoded == null) return;
        byte[] sealed = Base64.decode(encoded, Base64.NO_WRAP);
        if (sealed.length < IV_LENGTH + 16) throw new IOException("Sessão ilegível");
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE, key(false), new GCMParameterSpec(128, sealed, 0, IV_LENGTH));
        byte[] plain = cipher.doFinal(sealed, IV_LENGTH, sealed.length - IV_LENGTH);
        try {
            JSONObject json = new JSONObject(new String(plain, StandardCharsets.UTF_8));
            credentials = new Credentials(json.getString("key"), json.getString("secret"), json.getString("session"));
        } finally {
            java.util.Arrays.fill(plain, (byte) 0);
        }
    }
}
