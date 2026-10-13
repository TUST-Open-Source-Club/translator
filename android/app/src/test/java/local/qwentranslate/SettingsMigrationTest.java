package local.qwentranslate;
import org.junit.Test;
import java.util.Map;
import static org.junit.Assert.*;
public class SettingsMigrationTest {
 @Test public void modelAndLanguageSettingsSurviveMigration() {
  String text=SettingsMigration.exports(Map.of("QWEN_TRANSLATE_MODEL","fast","TRANSLATE_TO","ja"));
  assertTrue(text.contains("export QWEN_TRANSLATE_MODEL='fast'\n"));assertTrue(text.contains("export TRANSLATE_TO='ja'\n"));
 }
 @Test public void shellMetacharactersRemainLiteralAndInvalidNamesAreIgnored() {
  String text=SettingsMigration.exports(Map.of("MODEL","a'b$(touch never)\nnext","bad;command","value"));
  assertTrue(text.contains("export MODEL='a'\\''b$(touch never)\nnext'"));assertFalse(text.contains("bad;command"));
 }
}
