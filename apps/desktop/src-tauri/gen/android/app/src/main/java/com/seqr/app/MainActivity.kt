package com.seqr.app

import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import androidx.activity.enableEdgeToEdge
import com.google.firebase.messaging.FirebaseMessaging
import java.io.File

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    // Screen-capture protection (parity with the desktop screen-security feature): blocks
    // screenshots, screen recording, and the app's preview thumbnail. Enabled in release
    // only — a secure window renders black in scrcpy/mirroring, so debug builds stay
    // mirrorable for development (see scripts/android.sh --mirror).
    if (!BuildConfig.DEBUG) {
      window.setFlags(
        WindowManager.LayoutParams.FLAG_SECURE,
        WindowManager.LayoutParams.FLAG_SECURE,
      )
    }
    super.onCreate(savedInstanceState)

    // Fetch the FCM registration token and stash it in the app's files dir, where the Rust
    // core reads it (matrix_fcm_token) and registers a Matrix pusher after login.
    FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
      if (task.isSuccessful) {
        try {
          File(filesDir, "fcm_token").writeText(task.result)
          Log.i("Seqr", "FCM token stored in " + filesDir.absolutePath)
        } catch (e: Exception) {
          Log.w("Seqr", "failed to store FCM token", e)
        }
      } else {
        Log.w("Seqr", "FCM token fetch failed", task.exception)
      }
    }
  }
}
