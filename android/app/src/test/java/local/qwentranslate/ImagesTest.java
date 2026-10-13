package local.qwentranslate;
import org.junit.Test;
import static org.junit.Assert.*;
public class ImagesTest {
 @Test public void allPhotoFormatsReceiveTheSameDecodeBound() {
  assertEquals(1,Images.sampleSize(2048,2048));assertEquals(2,Images.sampleSize(2049,100));
  assertEquals(4,Images.sampleSize(8000,6250));assertEquals(8,Images.sampleSize(6250,10000));
 }
 @Test public void veryLargeMetadataCannotOverflowTheSampleCalculation() {
  int sample=Images.sampleSize(Integer.MAX_VALUE,1);assertTrue(((long)Integer.MAX_VALUE+sample-1)/sample<=2048);
 }
}
