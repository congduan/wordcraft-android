package ai.storyteller.wordcraft.android;

import android.app.NativeActivity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

/** Android-only host services for the Rust NativeActivity. */
public final class MainActivity extends NativeActivity {
    private static final int OPEN_DOCUMENT_REQUEST = 4107;
    private static final String PICKED_URI_FILE = "wordcraft-saf-picked-uri";
    private static final int MAX_OPEN_BYTES = 128 * 1024 * 1024;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
    }

    /** Called from Rust on Android's Java main thread. */
    public void startOpenDocument() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        intent.addFlags(Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, OPEN_DOCUMENT_REQUEST);
    }

    @Override
    @Deprecated
    protected void onActivityResult(int requestCode, int resultCode, Intent resultData) {
        super.onActivityResult(requestCode, resultCode, resultData);
        if (requestCode != OPEN_DOCUMENT_REQUEST || resultCode != RESULT_OK || resultData == null) {
            return;
        }
        Uri uri = resultData.getData();
        if (uri == null) {
            return;
        }
        try {
            int flags = resultData.getFlags() & Intent.FLAG_GRANT_READ_URI_PERMISSION;
            getContentResolver().takePersistableUriPermission(uri, flags);
        } catch (SecurityException ignored) {
            // Some providers do not offer persistable grants; the temporary read grant is enough.
        }
        writePickedUri(uri.toString());
    }

    /**
     * The URI remains in Java: Rust receives just owned bytes and never needs public-storage access.
     * A cap keeps a hostile content provider from exhausting native memory.
     */
    public byte[] readOpenDocument(String uriText) throws IOException {
        Uri uri = Uri.parse(uriText);
        try (InputStream input = getContentResolver().openInputStream(uri);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (input == null) {
                throw new IOException("The selected document could not be opened");
            }
            byte[] buffer = new byte[32 * 1024];
            int count;
            while ((count = input.read(buffer)) != -1) {
                if (count > MAX_OPEN_BYTES - output.size()) {
                    throw new IOException("The selected document is too large to open");
                }
                output.write(buffer, 0, count);
            }
            return output.toByteArray();
        }
    }

    private void writePickedUri(String uri) {
        File ready = new File(getFilesDir(), PICKED_URI_FILE);
        File temporary = new File(getFilesDir(), PICKED_URI_FILE + ".tmp");
        try (FileOutputStream output = new FileOutputStream(temporary, false)) {
            output.write(uri.getBytes(StandardCharsets.UTF_8));
            output.getFD().sync();
            if (!temporary.renameTo(ready)) {
                throw new IOException("Could not publish selected document");
            }
        } catch (IOException ignored) {
            // Rust will continue running; a later Open request can retry safely.
            temporary.delete();
        }
    }
}
