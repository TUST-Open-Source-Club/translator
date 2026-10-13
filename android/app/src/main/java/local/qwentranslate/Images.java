package local.qwentranslate;

import android.annotation.SuppressLint;
import android.content.ContentResolver;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Matrix;
import androidx.exifinterface.media.ExifInterface;
import android.net.Uri;
import android.webkit.MimeTypeMap;
import java.io.*;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.*;

/** Copies shared images; decoding is serialized and bounded independently of source format. */
final class Images {
    private static final List<String> NATIVE = Arrays.asList("jpg", "jpeg", "png", "webp", "gif", "bmp");
    private static final int MAX_SIDE = 2048;
    private static final long MAX_BYTES = 64L * 1024 * 1024;
    private static final long MAX_AGE_MS = 24 * 60 * 60 * 1000L;
    private static final ThreadPoolExecutor IMPORTS = new ThreadPoolExecutor(1, 1, 0,
            TimeUnit.MILLISECONDS, new ArrayBlockingQueue<>(4), r -> new Thread(r, "image-import"));

    private Images() {}
    static boolean execute(Runnable task) {
        try { IMPORTS.execute(task); return true; }
        catch (RejectedExecutionException full) { return false; }
    }
    static File dir(Context context) { return new File(context.getCacheDir(), "images"); }

    static File importImage(Context context, Uri uri) throws IOException {
        return importImage(context, uri, context.getContentResolver().getType(uri));
    }

    static File importImage(Context context, Uri uri, String mime) throws IOException {
        ContentResolver cr = context.getContentResolver();
        File dir = dir(context);
        if (!dir.isDirectory() && !dir.mkdirs()) throw new IOException("cannot create " + dir);
        String ext = mime == null ? null : MimeTypeMap.getSingleton().getExtensionFromMimeType(mime);
        BitmapFactory.Options bounds = new BitmapFactory.Options();
        bounds.inJustDecodeBounds = true;
        try (InputStream in = open(cr, uri)) { BitmapFactory.decodeStream(in, null, bounds); }
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) throw new IOException("unsupported image format");
        int sample = sampleSize(bounds.outWidth, bounds.outHeight);
        if (sample == 1 && ext != null && NATIVE.contains(ext)) {
            // Small native files retain EXIF; Rust applies orientation before cropping.
            File out = File.createTempFile("image-", "." + ext, dir);
            try {
                try (InputStream in = open(cr, uri); OutputStream os = new FileOutputStream(out)) {
                    byte[] buf = new byte[64 * 1024];
                    for (int n; (n = in.read(buf)) != -1;) os.write(buf, 0, n);
                }
                return out;
            } catch (IOException | RuntimeException e) { out.delete(); throw e; }
        }
        int orientation = ExifInterface.ORIENTATION_NORMAL;
        try (InputStream in = open(cr, uri)) {
            orientation = new ExifInterface(in).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL);
        } catch (IOException ignored) { /* formats such as BMP have no EXIF */ }
        BitmapFactory.Options opts = new BitmapFactory.Options();
        opts.inSampleSize = sample;
        Bitmap bitmap;
        try (InputStream in = open(cr, uri)) { bitmap = BitmapFactory.decodeStream(in, null, opts); }
        if (bitmap == null) throw new IOException("unsupported image format");
        File out = null;
        try {
            Matrix transform = orientationMatrix(orientation);
            if (!transform.isIdentity()) {
                Bitmap upright = Bitmap.createBitmap(bitmap, 0, 0, bitmap.getWidth(), bitmap.getHeight(), transform, true);
                if (upright != bitmap) { bitmap.recycle(); bitmap = upright; }
            }
            // Encoded pixels are upright; no stale EXIF can be applied a second time.
            out = File.createTempFile("image-", ".png", dir);
            try (OutputStream os = new FileOutputStream(out)) {
                if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, os)) throw new IOException("cannot encode image");
            }
            return out;
        } catch (IOException | RuntimeException | OutOfMemoryError e) {
            if (out != null) out.delete();
            throw e;
        } finally { bitmap.recycle(); }
    }

    static int sampleSize(int width, int height) {
        int sample = 1;
        while (((long) Math.max(width, height) + sample - 1) / sample > MAX_SIDE) sample *= 2;
        return sample;
    }

    private static Matrix orientationMatrix(int orientation) {
        Matrix m = new Matrix();
        switch (orientation) {
            case ExifInterface.ORIENTATION_FLIP_HORIZONTAL: m.setScale(-1, 1); break;
            case ExifInterface.ORIENTATION_ROTATE_180: m.setRotate(180); break;
            case ExifInterface.ORIENTATION_FLIP_VERTICAL: m.setScale(1, -1); break;
            case ExifInterface.ORIENTATION_TRANSPOSE: m.setRotate(90); m.postScale(-1, 1); break;
            case ExifInterface.ORIENTATION_ROTATE_90: m.setRotate(90); break;
            case ExifInterface.ORIENTATION_TRANSVERSE: m.setRotate(-90); m.postScale(-1, 1); break;
            case ExifInterface.ORIENTATION_ROTATE_270: m.setRotate(-90); break;
            default: break;
        }
        return m;
    }

    static void cleanUp(Context context) {
        File[] files = dir(context).listFiles();
        if (files == null) return;
        long now = System.currentTimeMillis();
        for (File f : files) if (now - f.lastModified() > MAX_AGE_MS) f.delete();
    }

    @SuppressLint("Recycle") // The returned wrapper owns and closes the resolver stream.
    private static InputStream open(ContentResolver cr, Uri uri) throws IOException {
        InputStream in = cr.openInputStream(uri);
        if (in == null) throw new IOException("cannot open " + uri);
        return new FilterInputStream(in) {
            long remaining = MAX_BYTES;
            @Override public int read() throws IOException {
                int value = super.read();
                if (value >= 0 && --remaining < 0) throw new IOException("image exceeds 64 MiB");
                return value;
            }
            @Override public int read(byte[] b, int off, int len) throws IOException {
                int n = this.in.read(b, off, (int) Math.min(len, remaining + 1));
                if (n > 0 && (remaining -= n) < 0) throw new IOException("image exceeds 64 MiB");
                return n;
            }
        };
    }
}
