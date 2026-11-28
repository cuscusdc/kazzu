package app.kazzu.player;

import android.content.ContentResolver;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;

import java.io.File;
import java.io.FileOutputStream;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayDeque;
import java.util.HashSet;
import java.util.Locale;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Copies SAF audio into the app's durable library so native decoders can seek it. */
final class LocalMusicImport {
    private static final String[] COLUMNS = {
        Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED
    };
    private static final Set<String> EXTENSIONS = new HashSet<>(java.util.Arrays.asList(
            "aac", "aif", "aiff", "alac", "flac", "m4a", "mp3", "mp4",
            "oga", "ogg", "opus", "wav", "wv", "lrc"));

    private static final class Directory {
        final String id;
        final File path;
        Directory(String id, File path) { this.id = id; this.path = path; }
    }

    static boolean mirror(Context context, Uri tree) throws IOException {
        ContentResolver resolver = context.getContentResolver();
        File root = new File(new File(context.getFilesDir(), "imported-music"), key(tree.toString()));
        File index = new File(new File(context.getFilesDir(), "imported-index"), key(tree.toString()));
        mkdir(root);
        mkdir(index);
        boolean changed = false;
        String rootId = DocumentsContract.getTreeDocumentId(tree);
        ArrayDeque<Directory> remaining = new ArrayDeque<>();
        remaining.add(new Directory(rootId, root));
        Set<String> seen = new HashSet<>();
        byte[] buffer = new byte[128 * 1024];
        while (!remaining.isEmpty()) {
            if (Thread.currentThread().isInterrupted()) throw new IOException("Importação interrompida");
            Directory dir = remaining.removeFirst();
            if (!seen.add(dir.id)) continue;
            Uri files = DocumentsContract.buildChildDocumentsUriUsingTree(tree, dir.id);
            try (Cursor cursor = resolver.query(files, COLUMNS, null, null, null)) {
                if (cursor == null) throw new IOException("A pasta não pode ser lida");
                while (cursor.moveToNext()) {
                    String id = cursor.getString(0);
                    String name = cursor.getString(1);
                    String mime = cursor.getString(2);
                    if (id == null || name == null) continue;
                    if (Document.MIME_TYPE_DIR.equals(mime)) {
                        remaining.add(new Directory(id, new File(root, safeName(name) + "-" + key(id))));
                        continue;
                    }
                    int dot = name.lastIndexOf('.');
                    if (dot < 0 || !EXTENSIONS.contains(name.substring(dot + 1).toLowerCase(Locale.ROOT))) continue;
                    long size = cursor.isNull(3) ? -1 : cursor.getLong(3);
                    long modified = cursor.isNull(4) ? 0 : cursor.getLong(4);
                    mkdir(dir.path);
                    String idHash = key(id);
                    String stem = name.substring(0, dot);
                    File destination = new File(dir.path, key(stem) + "-" + safeName(stem)
                            + name.substring(dot).toLowerCase(Locale.ROOT));
                    File stamp = new File(index, idHash + "-" + modified + "-" + size);
                    if (modified > 0 && stamp.isFile() && destination.isFile()) continue;
                    Uri document = DocumentsContract.buildDocumentUriUsingTree(tree, id);
                    copy(resolver, document, destination, size, buffer);
                    // A stamp is only written after a complete copy. Old stamps are
                    // removed so reverting a provider timestamp still refreshes the file.
                    File[] stamps = index.listFiles((parent, entry) -> entry.startsWith(idHash + "-"));
                    if (stamps != null) for (File old : stamps) if (!old.delete()) throw new IOException("Falha ao atualizar o índice de importação");
                    if (modified > 0 && !stamp.createNewFile()) throw new IOException("Falha ao registrar a importação");
                    changed = true;
                }
            }
        }
        return changed;
    }

    /** Finds byte-identical imports without changing or removing any files. */
    static long duplicateCopies(Context context) throws IOException {
        File root = new File(context.getFilesDir(), "imported-music");
        if (!root.isDirectory()) return 0;
        Map<Long, List<File>> bySize = new HashMap<>();
        ArrayDeque<File> directories = new ArrayDeque<>();
        directories.add(root);
        while (!directories.isEmpty()) {
            File[] files = directories.removeFirst().listFiles();
            if (files == null) continue;
            for (File file : files) {
                if (file.isDirectory()) {
                    directories.add(file);
                } else if (file.isFile()) {
                    String name = file.getName();
                    int dot = name.lastIndexOf('.');
                    if (dot < 0) continue;
                    String extension = name.substring(dot + 1).toLowerCase(Locale.ROOT);
                    if (!"lrc".equals(extension) && EXTENSIONS.contains(extension)) {
                        bySize.computeIfAbsent(file.length(), size -> new ArrayList<>()).add(file);
                    }
                }
            }
        }
        long duplicates = 0;
        byte[] buffer = new byte[128 * 1024];
        for (List<File> group : bySize.values()) {
            if (group.size() < 2) continue;
            Set<String> seen = new HashSet<>();
            for (File file : group) {
                MessageDigest digest = sha256();
                try (InputStream input = new FileInputStream(file)) {
                    int length;
                    while ((length = input.read(buffer)) != -1) {
                        if (Thread.currentThread().isInterrupted()) throw new IOException("Busca interrompida");
                        digest.update(buffer, 0, length);
                    }
                }
                if (!seen.add(hex(digest.digest()))) duplicates++;
            }
        }
        return duplicates;
    }

    private static void copy(ContentResolver resolver, Uri source, File destination, long size, byte[] buffer) throws IOException {
        File partial = File.createTempFile(".kazzu-", ".partial", destination.getParentFile());
        try {
            long copied = 0;
            try (InputStream input = resolver.openInputStream(source);
                 FileOutputStream output = new FileOutputStream(partial)) {
                if (input == null) throw new IOException("O arquivo não pode ser lido");
                int read;
                while ((read = input.read(buffer)) != -1) {
                    if (Thread.currentThread().isInterrupted()) throw new IOException("Importação interrompida");
                    output.write(buffer, 0, read);
                    copied += read;
                }
                output.getFD().sync();
            }
            if (size >= 0 && copied != size) throw new IOException("Arquivo mudou durante a importação");
            if (!partial.renameTo(destination)) throw new IOException("Não foi possível salvar o arquivo importado");
        } finally {
            partial.delete();
        }
    }

    private static void mkdir(File directory) throws IOException {
        if (!directory.isDirectory() && !directory.mkdirs() && !directory.isDirectory()) {
            throw new IOException("Não foi possível criar a pasta de importação");
        }
    }

    private static String safeName(String source) {
        String result = source.replaceAll("[^\\p{L}\\p{N} ._()-]", "_").trim();
        if (result.isEmpty()) return "audio";
        return result.substring(0, Math.min(48, result.length()));
    }

    private static MessageDigest sha256() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException error) {
            throw new IllegalStateException(error);
        }
    }

    private static String key(String value) {
        return hex(sha256().digest(value.getBytes(StandardCharsets.UTF_8)));
    }

    private static String hex(byte[] digest) {
        StringBuilder result = new StringBuilder(64);
        for (byte b : digest) {
            result.append(Character.forDigit((b >> 4) & 15, 16)).append(Character.forDigit(b & 15, 16));
        }
        return result.toString();
    }
}
