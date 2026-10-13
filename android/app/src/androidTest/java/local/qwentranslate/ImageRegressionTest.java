package local.qwentranslate;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import android.content.Context;
import android.graphics.*;
import androidx.exifinterface.media.ExifInterface;
import android.net.Uri;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.*;
import java.util.concurrent.*;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class ImageRegressionTest {
 private final Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();
 private File fixture(int width,int height,int color,Bitmap.CompressFormat format)throws Exception {
  File f=File.createTempFile("fixture-",".img",context.getCacheDir());Bitmap b=Bitmap.createBitmap(width,height,Bitmap.Config.RGB_565);b.eraseColor(color);
  try(OutputStream out=new FileOutputStream(f)){assertTrue(b.compress(format,100,out));}finally{b.recycle();}return f;
 }
 @Test public void concurrentNativeCopiesRemainDistinct()throws Exception {
  File red=fixture(24,16,Color.RED,Bitmap.CompressFormat.PNG),blue=fixture(24,16,Color.BLUE,Bitmap.CompressFormat.PNG);
  ExecutorService executor=Executors.newFixedThreadPool(2);File a=null,b=null;
  try {
   Future<File> first=executor.submit(()->Images.importImage(context,Uri.fromFile(red),"image/png"));
   Future<File> second=executor.submit(()->Images.importImage(context,Uri.fromFile(blue),"image/png"));
   a=first.get(5,TimeUnit.SECONDS);b=second.get(5,TimeUnit.SECONDS);assertNotEquals(a,b);
   Bitmap aa=BitmapFactory.decodeFile(a.getPath()),bb=BitmapFactory.decodeFile(b.getPath());
   try{assertEquals(Color.RED,aa.getPixel(0,0));assertEquals(Color.BLUE,bb.getPixel(0,0));}finally{aa.recycle();bb.recycle();}
  }finally{executor.shutdownNow();red.delete();blue.delete();if(a!=null)a.delete();if(b!=null)b.delete();}
 }
 @Test public void smallJpegRetainsExifForRust()throws Exception {
  File source=fixture(120,80,Color.RED,Bitmap.CompressFormat.JPEG),out=null;
  try {ExifInterface exif=new ExifInterface(source.getPath());exif.setAttribute(ExifInterface.TAG_ORIENTATION,"6");exif.saveAttributes();
   out=Images.importImage(context,Uri.fromFile(source),"image/jpeg");assertEquals(6,new ExifInterface(out.getPath()).getAttributeInt(ExifInterface.TAG_ORIENTATION,1));
  }finally{source.delete();if(out!=null)out.delete();}
 }
 @Test public void largeJpegIsSampledAndUprightWithoutASecondOrientation()throws Exception {
  File source=fixture(4096,3072,Color.RED,Bitmap.CompressFormat.JPEG),out=null;
  try {ExifInterface exif=new ExifInterface(source.getPath());exif.setAttribute(ExifInterface.TAG_ORIENTATION,"6");exif.saveAttributes();
   out=Images.importImage(context,Uri.fromFile(source),"image/jpeg");Bitmap result=BitmapFactory.decodeFile(out.getPath());
   try{assertEquals(1536,result.getWidth());assertEquals(2048,result.getHeight());}finally{result.recycle();}
   // PNG has no EXIF orientation tag; either undefined (0) or the normal value
   // is correct, and a stale Rotate90 value would apply the transform twice.
   int outputOrientation=new ExifInterface(out.getPath()).getAttributeInt(ExifInterface.TAG_ORIENTATION,0);
   assertTrue(outputOrientation==0 || outputOrientation==ExifInterface.ORIENTATION_NORMAL);
  }finally{source.delete();if(out!=null)out.delete();}
 }
 @Test public void allExifOrientationsPreserveCornerPositionsAfterSampling() throws Exception {
  int r=Color.RED,g=Color.GREEN,b=Color.BLUE,y=Color.YELLOW;
  int[][] expected={{r,g,b,y},{g,r,y,b},{y,b,g,r},{b,y,r,g},
                    {r,b,g,y},{b,r,y,g},{y,g,b,r},{g,y,r,b}};
  File source=File.createTempFile("orientation-", ".jpg", context.getCacheDir());
  Bitmap original=Bitmap.createBitmap(3072,2048,Bitmap.Config.RGB_565);
  Canvas canvas=new Canvas(original);Paint paint=new Paint();
  for(int corner=0;corner<4;corner++){
   paint.setColor(expected[0][corner]);int left=(corner%2)*1536,top=(corner/2)*1024;
   canvas.drawRect(left,top,left+1536,top+1024,paint);
  }
  try(OutputStream out=new FileOutputStream(source)){assertTrue(original.compress(Bitmap.CompressFormat.JPEG,100,out));}
  finally{original.recycle();}
  try {
   for(int orientation=1;orientation<=8;orientation++) {
    ExifInterface exif=new ExifInterface(source.getPath());
    exif.setAttribute(ExifInterface.TAG_ORIENTATION,Integer.toString(orientation));exif.saveAttributes();
    File out=Images.importImage(context,Uri.fromFile(source),"image/jpeg");
    Bitmap result=BitmapFactory.decodeFile(out.getPath());
    try {
     assertEquals(orientation<5?1536:1024,result.getWidth());
     assertEquals(orientation<5?1024:1536,result.getHeight());
     for(int corner=0;corner<4;corner++) {
      int color=result.getPixel(corner%2==0?10:result.getWidth()-11,corner/2==0?10:result.getHeight()-11);
      int wanted=expected[orientation-1][corner];
      assertTrue("orientation "+orientation+", corner "+corner,
        Math.abs(Color.red(color)-Color.red(wanted))<8 && Math.abs(Color.green(color)-Color.green(wanted))<8
         && Math.abs(Color.blue(color)-Color.blue(wanted))<8);
     }
    } finally {result.recycle();out.delete();}
   }
  } finally {source.delete();}
 }
}
