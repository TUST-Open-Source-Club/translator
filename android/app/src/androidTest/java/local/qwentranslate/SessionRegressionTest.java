package local.qwentranslate;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import android.app.Instrumentation;
import android.content.Context;
import com.termux.terminal.TerminalSession;
import org.junit.*;
import org.junit.runner.RunWith;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class SessionRegressionTest {
 private final Instrumentation instrumentation=InstrumentationRegistry.getInstrumentation();
 private Sessions sessions; private File rc; private byte[] saved;
 private void main(Runnable runnable){instrumentation.runOnMainSync(runnable);}
 private static byte[] read(File file)throws IOException {try(InputStream in=new FileInputStream(file);ByteArrayOutputStream out=new ByteArrayOutputStream()){byte[] b=new byte[4096];for(int n;(n=in.read(b))!=-1;)out.write(b,0,n);return out.toByteArray();}}
 private static void write(File file,String text)throws IOException {try(OutputStream out=new FileOutputStream(file)){out.write(text.getBytes(StandardCharsets.UTF_8));}}
 @Before public void before()throws Exception {
  main(()->{sessions=Sessions.get(instrumentation.getTargetContext());sessions.setListener(null);sessions.removeAll();});
  rc=new File(sessions.home(),".mkshrc");if(rc.exists())saved=read(rc);
  write(rc,"export QWEN_TRANSLATE_BIN=/system/bin/sh\nexport AUDIT_SETTING=restored\n");
 }
 @After public void after()throws Exception {
  main(()->{sessions.setListener(null);sessions.removeAll();});
  if(saved==null)rc.delete();else try(OutputStream out=new FileOutputStream(rc)){out.write(saved);}
 }
 private void finished(TerminalSession session)throws Exception {for(int i=0;i<100&&session.isRunning();i++)Thread.sleep(50);assertFalse("child did not finish",session.isRunning());}
 @Test public void startsWithoutViewAndLoadsSettingsWithLiteralArguments()throws Exception {
  File out=new File(sessions.home(),"regression-argv");out.delete();
  String literal="quotes' \" $(not_a_command); text";
  TerminalSession[] task={null};
  main(()->task[0]=sessions.startTranslate(Arrays.asList("-c","printf '%s:%s' \"$AUDIT_SETTING\" \"$1\" > \"$HOME/regression-argv\"","test",literal)));
  assertTrue(task[0].getPid()>0);assertNotNull(task[0].getEmulator());finished(task[0]);
  assertEquals("restored:"+literal,new String(read(out),StandardCharsets.UTF_8));out.delete();
 }
 @Test public void replacingOnlyFinishedTaskNeverPublishesAnEmptyCollection()throws Exception {
  TerminalSession[] task={null};main(()->task[0]=sessions.startTranslate(Arrays.asList("-c","exit 0")));finished(task[0]);
  List<Integer> sizes=new ArrayList<>();
  main(()->{sessions.setListener(new Sessions.Listener(){public void onSessionsChanged(){sizes.add(sessions.size());}public void onScreenUpdated(TerminalSession s){}});sessions.startTranslate(Arrays.asList("-c","exit 0"));});
  assertFalse(sizes.contains(0));assertEquals(1,sessions.size());
 }
 @Test public void initialShellSourcesConfigOnceAndRemainsUsable()throws Exception {
  File marker=new File(sessions.home(),"regression-start");marker.delete();
  write(rc,"export QWEN_TRANSLATE_BIN=/system/bin/true\nprintf x >> \"$HOME/regression-start\"\n");
  TerminalSession[] shell={null};main(()->shell[0]=sessions.startShell(true));
  for(int i=0;i<100&&!marker.exists();i++)Thread.sleep(50);
  Thread.sleep(100);assertEquals("x",new String(read(marker),StandardCharsets.UTF_8));assertTrue(shell[0].isRunning());marker.delete();
 }
}
