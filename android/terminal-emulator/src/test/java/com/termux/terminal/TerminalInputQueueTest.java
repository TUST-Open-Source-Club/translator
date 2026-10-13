package com.termux.terminal;
import org.junit.Test;
import java.util.Arrays;
import java.util.concurrent.*;
import static org.junit.Assert.*;
public class TerminalInputQueueTest {
 @Test(timeout=1000) public void longPasteNeverWaitsForTerminalReader() {
  TerminalInputQueue q=new TerminalInputQueue(); byte[] paste=new byte[128*1024];Arrays.fill(paste,(byte)'x');
  assertTrue(q.offer(paste,0,paste.length)); byte[] received=new byte[paste.length];
  assertEquals(paste.length,q.read(received).count);assertArrayEquals(paste,received);
 }
 @Test public void fullQueueRejectsWholeInputAndRecoversAfterDrain() {
  TerminalInputQueue q=new TerminalInputQueue();byte[] maximum=new byte[TerminalInputQueue.MAX_BYTES];
  assertTrue(q.offer(maximum,0,maximum.length));assertFalse(q.offer(new byte[]{42},0,1));
  assertEquals(maximum.length,q.read(maximum).count);assertTrue(q.offer(new byte[]{7},0,1));
  byte[] out=new byte[1];assertEquals(1,q.read(out).count);assertEquals(7,out[0]);
 }
 @Test public void cancellationDiscardsPendingInputButKeepsLaterKeystrokes() {
  TerminalInputQueue q=new TerminalInputQueue();q.offer(new byte[]{1,2,3},0,3);
  TerminalInputQueue.Read inFlight=q.read(new byte[1]);q.clear();q.offer(new byte[]{9},0,1);
  byte[] out=new byte[8];TerminalInputQueue.Read later=q.read(out);
  assertNotEquals(inFlight.generation,later.generation);assertEquals(q.generation(),later.generation);
  assertEquals(1,later.count);assertEquals(9,out[0]);
 }
 @Test(timeout=1000) public void closeReleasesWaitingWriter() throws Exception {
  TerminalInputQueue q=new TerminalInputQueue();ExecutorService executor=Executors.newSingleThreadExecutor();
  try {Future<TerminalInputQueue.Read> waiting=executor.submit(()->q.read(new byte[8]));q.close();assertNull(waiting.get(500,TimeUnit.MILLISECONDS));assertFalse(q.offer(new byte[]{1},0,1));}
  finally{executor.shutdownNow();}
 }
 @Test public void emptyClipboardIsNoOp() {TerminalInputQueue q=new TerminalInputQueue();assertTrue(q.offer(new byte[0],0,0));q.close();assertNull(q.read(new byte[1]));}
 @Test public void manySmallKeystrokesAreCoalescedWithoutChangingBytes() {
  TerminalInputQueue q = new TerminalInputQueue();
  byte[] expected = new byte[128 * 1024];
  for (int i = 0; i < expected.length; i++) { expected[i] = (byte) i; assertTrue(q.offer(expected, i, 1)); }
  byte[] actual = new byte[expected.length]; int offset = 0;
  while (offset < actual.length) offset += q.read(actual).count;
  assertArrayEquals(expected, actual);
 }
 @Test public void bracketedPasteIsRejectedWithoutLeavingPartialDelimiters() {
  TerminalInputQueue q = new TerminalInputQueue();
  byte[] pending = new byte[TerminalInputQueue.MAX_BYTES - 8];
  Arrays.fill(pending, (byte) 'x');
  assertTrue(q.offer(pending, 0, pending.length));
  TerminalTestCase.MockTerminalOutput output = new TerminalTestCase.MockTerminalOutput() {
   @Override public void write(byte[] data, int offset, int count) { q.offer(data, offset, count); }
  };
  TerminalEmulator terminal = new TerminalEmulator(output, 80, 24, 8, 16, 100, null);
  byte[] enable = "\033[?2004h".getBytes(java.nio.charset.StandardCharsets.UTF_8);
  terminal.append(enable, enable.length);
  terminal.paste("hello");
  byte[] received = new byte[pending.length];
  assertEquals(pending.length, q.read(received).count);
  assertArrayEquals(pending, received);
  // Only this subsequent keystroke may remain; a leaked start/end delimiter
  // would instead be the next queue entry.
  assertTrue(q.offer(new byte[] {42}, 0, 1));
  byte[] next = new byte[32];
  assertEquals(1, q.read(next).count);
  assertEquals(42, next[0]);
 }

 @Test public void oversizedPasteIsRejectedBeforeItCanBeWritten() {
  final int[] rejected = {0};
  final String[] written = {null};
  TerminalTestCase.MockTerminalOutput output = new TerminalTestCase.MockTerminalOutput() {
   @Override public void writeText(String prefix, String text, String suffix) {
    written[0] = prefix + text + suffix;
   }
   @Override public void rejectInput() { rejected[0]++; }
  };
  TerminalEmulator terminal = new TerminalEmulator(output, 80, 24, 8, 16, 100, null);
  StringBuilder huge = new StringBuilder(TerminalInputQueue.MAX_BYTES + 1);
  for (int i = 0; i <= TerminalInputQueue.MAX_BYTES; i++) huge.append('x');
  terminal.paste(huge.toString());
  assertEquals(1, rejected[0]);
  assertNull(written[0]);
 }
}
