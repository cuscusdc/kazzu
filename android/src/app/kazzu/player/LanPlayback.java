package app.kazzu.player;

import android.util.Base64;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.NetworkInterface;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.SecureRandom;
import java.util.Arrays;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.TimeUnit;
import java.util.function.IntConsumer;
import java.util.function.Consumer;
import java.util.function.Supplier;

import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/** A user-started, short-lived private LAN control channel. */
final class LanPlayback implements AutoCloseable {
    private static final byte VERSION = 1;
    private static final int KEY_BYTES = 32;
    private static final int NONCE_BYTES = 12;
    private static final int TAG_BYTES = 16;
    private static final int TIMEOUT_MS = 1500;
    private static final SecureRandom RANDOM = new SecureRandom();
    private static final long LIFETIME = TimeUnit.MINUTES.toNanos(30);
    private static final int MAX_COMMANDS = 2400;
    private static final int LISTEN_TOGETHER = 4;
    private static final int SUGGEST_TRACK = 5;
    private static final int BACKUP_CHUNK = 6;
    private static final int CHUNK_BYTES = 32 * 1024;
    private static final int MAX_BACKUP = 8 * 1024 * 1024;
    private static final int MAX_REQUEST = 4096;
    private static final int MAX_RESPONSE = CHUNK_BYTES + 64;

    static final class Now {
        final String id;
        final String title;
        final String artist;
        final long position;
        final boolean playing;
        final long observed = System.nanoTime();

        Now(String id, String title, String artist, long position, boolean playing) {
            this.id = id;
            this.title = title;
            this.artist = artist;
            this.position = position;
            this.playing = playing;
        }

        long currentPosition() {
            return position + (playing ? TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - observed) : 0);
        }
    }

    private final ServerSocket server;
    private final String code;
    private final Thread thread;
    private volatile boolean closed;

    LanPlayback(IntConsumer command, Supplier<Now> playback, Consumer<Now> suggest, Supplier<byte[]> backup, Consumer<LanPlayback> expired) throws IOException {
        Inet4Address address = lanAddress();
        byte[] key = new byte[KEY_BYTES];
        RANDOM.nextBytes(key);
        server = new ServerSocket();
        try {
            server.bind(new InetSocketAddress(address, 0));
            server.setSoTimeout(TIMEOUT_MS);
        } catch (IOException error) {
            server.close();
            throw error;
        }
        byte[] payload = ByteBuffer.allocate(1 + 4 + 2 + KEY_BYTES)
                .put(VERSION).put(address.getAddress()).putShort((short) server.getLocalPort()).put(key).array();
        code = Base64.encodeToString(payload, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
        Arrays.fill(payload, (byte) 0);
        thread = new Thread(() -> serve(key, command, playback, suggest, backup, expired), "kazzu-lan-control");
        thread.start();
    }

    String code() { return code; }

    private static Inet4Address lanAddress() throws IOException {
        Inet4Address fallback = null;
        Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
        while (interfaces != null && interfaces.hasMoreElements()) {
            NetworkInterface network = interfaces.nextElement();
            if (!network.isUp() || network.isLoopback() || network.isPointToPoint()) continue;
            Enumeration<InetAddress> addresses = network.getInetAddresses();
            while (addresses.hasMoreElements()) {
                InetAddress address = addresses.nextElement();
                if (!(address instanceof Inet4Address) || !address.isSiteLocalAddress()) continue;
                if (network.getName().startsWith("wlan") || network.getName().startsWith("eth")) {
                    return (Inet4Address) address;
                }
                if (fallback == null && (network.getName().startsWith("ap") || network.getName().startsWith("usb"))) fallback = (Inet4Address) address;
            }
        }
        if (fallback != null) return fallback;
        throw new IOException("Rede local IPv4 indisponível");
    }

    private void serve(byte[] key, IntConsumer command, Supplier<Now> playback, Consumer<Now> suggest, Supplier<byte[]> backup, Consumer<LanPlayback> expired) {
        long started = System.nanoTime();
        Set<String> used = new HashSet<>();
        try {
            while (!closed && System.nanoTime() - started < LIFETIME && used.size() < MAX_COMMANDS) {
                try (Socket socket = server.accept()) {
                    socket.setSoTimeout(TIMEOUT_MS);
                    if (!socket.getInetAddress().isSiteLocalAddress()) continue;
                    DataInputStream input = new DataInputStream(socket.getInputStream());
                    byte[] requestNonce = read(input, NONCE_BYTES);
                    int size = input.readUnsignedShort();
                    if (size < 1 + TAG_BYTES || size > MAX_REQUEST) continue;
                    byte[] ciphertext = read(input, size);
                    byte[] plain = crypt(Cipher.DECRYPT_MODE, key, requestNonce, (byte) 1, ciphertext);
                    DataInputStream request = new DataInputStream(new ByteArrayInputStream(plain));
                    int action = request.readUnsignedByte();
                    if (action > BACKUP_CHUNK || !used.add(Arrays.toString(requestNonce))) continue;
                    ByteArrayOutputStream reply = new ByteArrayOutputStream();
                    DataOutputStream response = new DataOutputStream(reply);
                    response.write(requestNonce);
                    if (action == LISTEN_TOGETHER) {
                        Now now = playback.get();
                        response.writeBoolean(now != null);
                        if (now != null) {
                            response.writeUTF(now.id);
                            response.writeUTF(truncate(now.title));
                            response.writeUTF(truncate(now.artist));
                            response.writeLong(now.currentPosition());
                            response.writeBoolean(now.playing);
                        }
                    } else if (action == SUGGEST_TRACK) {
                        String id = request.readUTF();
                        String title = request.readUTF();
                        String artist = request.readUTF();
                        boolean allowed = suggest != null && id.matches("[A-Za-z0-9_-]{1,32}")
                                && !title.isEmpty() && title.length() <= 256 && artist.length() <= 256;
                        if (allowed) suggest.accept(new Now(id, title, artist, 0, false));
                        response.writeBoolean(allowed);
                    } else if (action == BACKUP_CHUNK) {
                        int offset = request.readInt();
                        byte[] snapshot = backup == null ? null : backup.get();
                        if (snapshot == null || snapshot.length > MAX_BACKUP || offset < 0 || offset > snapshot.length
                                || offset % CHUNK_BYTES != 0) {
                            response.writeInt(-1);
                        } else {
                            response.writeInt(snapshot.length);
                            response.write(snapshot, offset, Math.min(CHUNK_BYTES, snapshot.length - offset));
                        }
                    } else {
                        if (command == null) continue;
                        command.accept(action);
                    }
                    byte[] responseNonce = nonce();
                    byte[] encrypted = crypt(Cipher.ENCRYPT_MODE, key, responseNonce, (byte) 2, reply.toByteArray());
                    DataOutputStream output = new DataOutputStream(socket.getOutputStream());
                    output.write(responseNonce);
                    output.writeShort(encrypted.length);
                    output.write(encrypted);
                    output.flush();
                } catch (IOException | GeneralSecurityException ignored) {
                    // Connections without a valid capability never reach the player.
                }
            }
        } finally {
            Arrays.fill(key, (byte) 0);
            boolean notifyExpired = !closed;
            close();
            if (notifyExpired) expired.accept(this);
        }
    }

    static void send(String code, int command) throws IOException, GeneralSecurityException {
        if (command < 0 || command >= LISTEN_TOGETHER) throw new IOException("Comando inválido");
        request(code, new byte[] { (byte) command });
    }

    static Now sync(String code) throws IOException, GeneralSecurityException {
        long started = System.nanoTime();
        DataInputStream response = new DataInputStream(new ByteArrayInputStream(request(code, new byte[] { LISTEN_TOGETHER })));
        if (!response.readBoolean()) return null;
        String id = response.readUTF();
        String title = response.readUTF();
        String artist = response.readUTF();
        long position = response.readLong();
        boolean playing = response.readBoolean();
        long transit = Math.min(750, TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started) / 2);
        return new Now(id, title, artist, Math.max(0, position) + (playing ? transit : 0), playing);
    }

    static boolean suggest(String code, Now now) throws IOException, GeneralSecurityException {
        ByteArrayOutputStream payload = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(payload);
        output.writeByte(SUGGEST_TRACK);
        output.writeUTF(now.id);
        output.writeUTF(truncate(now.title));
        output.writeUTF(truncate(now.artist));
        byte[] response = request(code, payload.toByteArray());
        return response.length == 1 && response[0] == 1;
    }

    static String copyBackup(String code) throws IOException, GeneralSecurityException {
        ByteArrayOutputStream backup = new ByteArrayOutputStream();
        int length = -1;
        do {
            int offset = backup.size();
            byte[] request = ByteBuffer.allocate(5).put((byte) BACKUP_CHUNK).putInt(offset).array();
            byte[] response = request(code, request);
            if (response.length < Integer.BYTES) throw new IOException("Resposta inválida");
            int total = ByteBuffer.wrap(response).getInt();
            if (total < 0 || total > MAX_BACKUP || offset > total ||
                    (length >= 0 && length != total) ||
                    response.length - Integer.BYTES != Math.min(CHUNK_BYTES, total - offset)) {
                throw new IOException("O anfitrião não disponibilizou um backup");
            }
            length = total;
            backup.write(response, Integer.BYTES, response.length - Integer.BYTES);
        } while (backup.size() < length);
        return new String(backup.toByteArray(), StandardCharsets.UTF_8);
    }

    private static byte[] request(String code, byte[] payloadBytes) throws IOException, GeneralSecurityException {
        if (code.length() != 52) throw new IOException("Código inválido");
        byte[] payload;
        try {
            payload = Base64.decode(code, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
        } catch (IllegalArgumentException error) {
            throw new IOException("Código inválido");
        }
        if (payload.length != 1 + 4 + 2 + KEY_BYTES || payload[0] != VERSION) {
            throw new IOException("Código inválido");
        }
        ByteBuffer decoded = ByteBuffer.wrap(payload);
        decoded.get();
        byte[] ip = new byte[4];
        decoded.get(ip);
        InetAddress address = InetAddress.getByAddress(ip);
        int port = Short.toUnsignedInt(decoded.getShort());
        if (!address.isSiteLocalAddress() || port < 1024) throw new IOException("Endereço fora da rede local");
        byte[] key = new byte[KEY_BYTES];
        decoded.get(key);
        Arrays.fill(payload, (byte) 0);
        try (Socket socket = new Socket()) {
            socket.connect(new InetSocketAddress(address, port), TIMEOUT_MS);
            socket.setSoTimeout(TIMEOUT_MS);
            byte[] requestNonce = nonce();
            DataOutputStream output = new DataOutputStream(socket.getOutputStream());
            byte[] encrypted = crypt(Cipher.ENCRYPT_MODE, key, requestNonce, (byte) 1, payloadBytes);
            output.write(requestNonce);
            output.writeShort(encrypted.length);
            output.write(encrypted);
            output.flush();
            DataInputStream input = new DataInputStream(socket.getInputStream());
            byte[] responseNonce = read(input, NONCE_BYTES);
            int length = input.readUnsignedShort();
            if (length < NONCE_BYTES + TAG_BYTES || length > MAX_RESPONSE) throw new IOException("Resposta inválida");
            byte[] response = crypt(Cipher.DECRYPT_MODE, key, responseNonce, (byte) 2, read(input, length));
            if (response.length < NONCE_BYTES
                    || !Arrays.equals(Arrays.copyOf(response, NONCE_BYTES), requestNonce)) {
                throw new IOException("Resposta inválida");
            }
            return Arrays.copyOfRange(response, NONCE_BYTES, response.length);
        } finally {
            Arrays.fill(key, (byte) 0);
        }
    }

    private static String truncate(String text) {
        return text.substring(0, Math.min(256, text.length()));
    }

    private static byte[] nonce() {
        byte[] nonce = new byte[NONCE_BYTES];
        RANDOM.nextBytes(nonce);
        return nonce;
    }

    private static byte[] read(DataInputStream stream, int size) throws IOException {
        byte[] bytes = new byte[size];
        stream.readFully(bytes);
        return bytes;
    }

    private static byte[] crypt(int operation, byte[] key, byte[] nonce, byte direction, byte[] bytes)
            throws GeneralSecurityException {
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(operation, new SecretKeySpec(key, "AES"), new GCMParameterSpec(TAG_BYTES * 8, nonce));
        cipher.updateAAD("kazzu-playback".getBytes(StandardCharsets.US_ASCII));
        cipher.updateAAD(new byte[] { VERSION, direction });
        return cipher.doFinal(bytes);
    }

    @Override public void close() {
        closed = true;
        try { server.close(); } catch (IOException ignored) {}
    }
}
