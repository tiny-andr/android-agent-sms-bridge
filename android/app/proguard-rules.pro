# Keep NanoHTTPD handlers (reflection-free, but keep for safety)
-keep class fi.iki.elonen.** { *; }
-keepclassmembers class * extends fi.iki.elonen.NanoHTTPD { *; }
