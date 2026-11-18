# Die Brücke zur Web-Oberfläche wird per Reflection aus JavaScript aufgerufen
-keepclassmembers class ch.zweers.nrzmusic.Bridge {
    @android.webkit.JavascriptInterface <methods>;
}
-keepattributes JavascriptInterface
